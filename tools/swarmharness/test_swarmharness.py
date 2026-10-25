"""Offline conformance for the kernel: config survival semantics + index."""
import sys, tempfile, json
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent))
import swarmconfig as sc
import empire_index as ei

tmp = Path(tempfile.mkdtemp())

# 1. valid config -> ok + last-good written
good = tmp / "swarmconfig.yaml"
SEED = """schema_version: 1
kernel: {repo: ~/gh/yggterm, policy_dir: ~/.yggterm/dream/policy, feedback: ~/.yggterm/dream/audit-feedback.json, state_dir: ~/.yggterm/dream, wake_interval_min: 20}
intelligence: {primary: {model: gpt-6.1-sol, harness: codex}, fallback: {model: gemini-3.8-flash, harness: agy}, degraded_mode: deterministic-only}
orchestrators: {level0: {name: dream-orchestrator, host: dev}, sub_orchestrators: []}
planes: {zai-coding-plan: {state: active}, autoclaw: {state: walled}}
intent: {canonical: x}
"""
good.write_text(SEED)
cfg, note = sc.load(good)
assert cfg and note == "ok", note
assert good.with_suffix(".yaml.last-good").exists()

# 2. broken edit -> last-good survives
good.write_text("schema_version: 99\nkernel: {broken")
cfg2, note2 = sc.load(good)
assert cfg2 and "last-good" in note2, note2

# 3. newer major fails closed
err = sc.validate({"schema_version": 9, "kernel": {}, "intelligence": {}, "orchestrators": {}, "planes": {}})
assert err and "newer than kernel" in err, err

# 4. missing section fails
err2 = sc.validate({"schema_version": 1, "kernel": {}})
assert err2 and "missing section" in err2, err2

# 5. empire index builds from sparse inputs
idx = ei.build(cfg, {"namespaces": [{"ns": "-home-x", "per_day": 2}]},
               None, str(tmp), str(tmp / "empire-index.md"))
assert "empire index" in idx and "UNKNOWN" in idx and "audit findings" in idx
assert (tmp / "empire-index.md").exists()

# 6. index with real queue rows
idx2 = ei.build(cfg, {"namespaces": []},
                [{"route": "zai-coding-plan", "status": "running"},
                 {"route": "autoclaw", "status": "failed"}],
                str(tmp), str(tmp / "empire-index.md"))
assert "zai-coding-plan running: 1" in idx2 and "autoclaw failed: 1" in idx2

print("KERNEL CONFORMANCE PASS (6 checks)")
