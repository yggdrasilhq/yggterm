#!/usr/bin/env python3
"""[T-002 m1+m3] MoE compression routing + cross-layer sweep emission.

    python3 tests/test_dream_moe_routing.py [path-to-ygg-memory.py]

The charter amendment (owner 2026-10-17): consolidation routes each knowledge
type to its EXPERT treatment — deterministic/derivable → code, System-1
semantic → Jev, deep synthesis → the composer, legal/IP/evidentiary → the
legal consultant (owner-gated). The brief carries the doctrine; claims carry
the optional route; validation NEVER lets a legal route dress a synthesized
conclusion as proven (route legal ⇒ proof claimed). Cross-layer pattern
recognition rides SWEEP CONTEXT: other recently-dreamed namespaces' rollup
excerpts enter the brief (deterministic selection, bounded bytes) and a claim
observed across >= 2 namespaces may mint as scope=cross-layer with layers.
"""

import datetime
import importlib.util
import json
import sys
import tempfile
from pathlib import Path

MODULE_PATH = Path(__file__).resolve().parent.parent / "ygg-memory.py"


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_under_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Args:
    def __init__(self, **kw):
        self.subcommand = "dream"
        self.root = None
        self.harness = "zcode"
        self.ns = None
        self.json = False
        self.prepare = False
        self.apply = False
        self.run = False
        self.status = False
        self.phase = "synthesis"
        self.extra_input = []
        self.budget = None
        self.dry_run = False
        self.composer = "sol"
        self.model = "gpt-6.1-sol"
        self.effort = "high"
        self.codex_bin = None
        self.force = False
        self.quiet = False
        self.__dict__.update(kw)


def journal_door(module, root, ns, filename, content):
    ns_dir = module.get_namespace_dir(root, ns)
    ns_dir.mkdir(parents=True, exist_ok=True)
    (ns_dir / filename).write_text(content, encoding="utf-8")
    module.append_journal_entry(root, ns, filename, "door", "update", "zcode", f"test {filename}")


GOOD_NOW = """## §CURRENT_STATUS
- moe slice landed [as-of: 2026-10-17; proof: deployed]

## §NEXT_UNITS
- next queued [as-of: 2026-10-17; proof: claimed]

## §ACTIVE_TRAPS
- none live [as-of: 2026-10-17; proof: measured-live]

## §POINTERS
- door-a.md
"""

GOOD_CRITIC = """=== VERDICTS ===
[
  {"claim_id": "c1", "verdict": "upheld", "counter_example": null},
  {"claim_id": "c2", "verdict": "upheld", "counter_example": null},
  {"claim_id": "c3", "verdict": "upheld", "counter_example": null}
]
"""


def response(claims_json: str) -> str:
    return f"""=== NOW.MD ===
{GOOD_NOW}
=== OPS ===
[]
=== CLAIMS ===
{claims_json}
=== OWNER-NOW ===
null
"""


def claims(entries) -> str:
    return json.dumps(entries, indent=1)


def base_claim(cid="c1", **over):
    claim = {
        "claim_id": cid,
        "text": "moe slice landed",
        "as_of": "2026-10-17",
        "proof": "deployed",
        "source_refs": ["seq#1/door-a.md"],
    }
    claim.update(over)
    return claim


def test_brief_carries_moe_doctrine(module, failures):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        ns = "-t-moe"
        journal_door(module, root, ns, "door-a.md", "# a\n\nalpha\n")
        staged = module.prepare_dream(root, ns, "synthesis", [], 10 ** 9)
        if staged is None:
            failures.append("moe: prepare staged nothing")
            return
        brief_path, manifest = staged
        brief = brief_path.read_text(encoding="utf-8")
        if "EXPERT ROUTING (MoE" not in brief:
            failures.append("moe: brief missing the EXPERT ROUTING section")
        for token in ('route "code"', 'route "jev"', 'route "composer"', 'route "legal"'):
            if token not in brief:
                failures.append(f"moe: brief missing routing lane {token}")
        if manifest.get("moe_routing", {}).get("doctrine") is not True:
            failures.append("moe: manifest does not record the routing doctrine")


def test_sweep_context_selects_recently_dreamed(module, failures):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        ns = "-t-moe"
        other = "-t-sweep-other"
        journal_door(module, root, ns, "door-a.md", "# a\n\nalpha\n")
        now_iso = datetime.datetime.now(datetime.timezone.utc).isoformat()
        other_dir = module.get_dream_dir(root, other)
        (other_dir / "watermark.json").write_text(
            json.dumps({"ns": other, "last_seq": 3, "last_dream_ts": now_iso}), encoding="utf-8"
        )
        other_ns = module.get_namespace_dir(root, other)
        (other_ns / "NOW.md").write_text(
            "## §CURRENT_STATUS\n- sweep twin pattern observed [as-of: 2026-10-17; proof: claimed]\n\n"
            "## §NEXT_UNITS\n- x [as-of: 2026-10-17; proof: claimed]\n",
            encoding="utf-8",
        )
        staged = module.prepare_dream(root, ns, "synthesis", [], 10 ** 9)
        brief = staged[0].read_text(encoding="utf-8")
        manifest = staged[1]
        if "SWEEP CONTEXT" not in brief:
            failures.append("sweep: brief missing SWEEP CONTEXT despite a recent other-ns dream")
            return
        if "[sweep twin pattern observed]" not in brief and "sweep twin pattern" not in brief:
            failures.append("sweep: the other namespace's §CURRENT_STATUS did not ride the brief")
        if manifest.get("sweep_context", {}).get("namespaces") != [other]:
            failures.append(f"sweep: manifest namespace list wrong: {manifest.get('sweep_context')}")
        # a stale dream (beyond the window) must NOT ride
        stale_dir = module.get_dream_dir(root, "-t-sweep-stale")
        (stale_dir / "watermark.json").write_text(
            json.dumps({"ns": "-t-sweep-stale", "last_seq": 1, "last_dream_ts": "2020-01-01T00:00:00+00:00"}),
            encoding="utf-8",
        )
        staged2 = module.prepare_dream(root, ns, "synthesis", [], 10 ** 9)
        manifest2 = staged2[1]
        if "-t-sweep-stale" in manifest2["sweep_context"]["namespaces"]:
            failures.append("sweep: a years-stale dream rode the sweep context")


def test_route_validation(module, failures):
    ns_dir = Path(tempfile.mkdtemp()) / "ns"
    ns_dir.mkdir(parents=True)
    # route legal + proof deployed = the synthesized-conclusion laundering hole
    parsed = {
        "now_md": GOOD_NOW,
        "ops": [],
        "claims": [base_claim(route="legal", proof="deployed")],
        "owner_now": None,
    }
    errors = module.validate_dream_response(parsed, [], ns_dir)
    if not any("route=legal requires proof 'claimed'" in e for e in errors):
        failures.append(f"moe: legal route with deployed proof not refused: {errors}")
    # route legal + proof claimed passes
    parsed["claims"] = [base_claim(route="legal", proof="claimed")]
    errors = module.validate_dream_response(parsed, [], ns_dir)
    if any("route" in e for e in errors):
        failures.append(f"moe: legal+claimed wrongly refused: {errors}")
    # unknown route refused
    parsed["claims"] = [base_claim(route="vibes")]
    errors = module.validate_dream_response(parsed, [], ns_dir)
    if not any("route not in vocabulary" in e for e in errors):
        failures.append(f"moe: unknown route not refused: {errors}")
    # jev / code / composer pass
    for route in ("jev", "code", "composer"):
        parsed["claims"] = [base_claim(route=route)]
        errors = module.validate_dream_response(parsed, [], ns_dir)
        if any("route" in e for e in errors):
            failures.append(f"moe: route {route} wrongly refused: {errors}")


def test_cross_layer_validation(module, failures):
    ns_dir = Path(tempfile.mkdtemp()) / "ns"
    ns_dir.mkdir(parents=True)
    parsed = {"now_md": GOOD_NOW, "ops": [], "claims": [], "owner_now": None}
    # one layer is not cross-layer
    parsed["claims"] = [base_claim(scope="cross-layer", layers=["-only-one"])]
    errors = module.validate_dream_response(parsed, [], ns_dir)
    if not any(">= 2 layers" in e for e in errors):
        failures.append(f"cross-layer: single-layer claim not refused: {errors}")
    # bogus scope refused
    parsed["claims"] = [base_claim(scope="per-ns", layers=["a", "b"])]
    errors = module.validate_dream_response(parsed, [], ns_dir)
    if not any("scope must be 'cross-layer'" in e for e in errors):
        failures.append(f"cross-layer: bogus scope not refused: {errors}")
    # proper cross-layer passes
    parsed["claims"] = [base_claim(scope="cross-layer", layers=["-t-a", "-t-b"])]
    errors = module.validate_dream_response(parsed, [], ns_dir)
    if any("cross-layer" in e or "scope" in e or "layers" in e for e in errors):
        failures.append(f"cross-layer: proper claim wrongly refused: {errors}")


def test_merge_claims_persists_routing(module, failures):
    state = {"claims": {}}
    state = module.merge_claims(
        state,
        [
            base_claim(route="jev"),
            base_claim(cid="c2", text="twin pattern", proof="claimed",
                       scope="cross-layer", layers=["-t-a", "-t-b"]),
        ],
        "2026-10-17",
    )
    if state["claims"]["c1"].get("route") != "jev":
        failures.append(f"merge: route not persisted: {state['claims']['c1']}")
    c2 = state["claims"]["c2"]
    if c2.get("scope") != "cross-layer" or c2.get("layers") != ["-t-a", "-t-b"]:
        failures.append(f"merge: cross-layer scope/layers not persisted: {c2}")
    # a claim with no route stays clean
    state2 = module.merge_claims({"claims": {}}, [base_claim(cid="c3")], "2026-10-17")
    if "route" in state2["claims"]["c3"]:
        failures.append("merge: unrouteed claim grew a route field")


def main() -> int:
    module = load_module(Path(sys.argv[1]) if len(sys.argv) > 1 else MODULE_PATH)
    failures: list[str] = []
    test_brief_carries_moe_doctrine(module, failures)
    test_sweep_context_selects_recently_dreamed(module, failures)
    test_route_validation(module, failures)
    test_cross_layer_validation(module, failures)
    test_merge_claims_persists_routing(module, failures)
    if failures:
        print(f"dream-moe-routing tests: {len(failures)} FAILURE(S)")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("dream-moe-routing tests: all green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
