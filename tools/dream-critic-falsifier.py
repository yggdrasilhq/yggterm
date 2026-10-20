#!/usr/bin/env python3
"""[T-027] The planted-hallucination falsifier — cross-model critic proof.

    python3 tools/dream-critic-falsifier.py [--live] [--timeout 600]
    env: FALSIFIER_SAME_MODEL (default opencode/glm-5.3-flash-free)
         FALSIFIER_CROSS_MODEL (default gpt-6.1-sol)

The T-027 falsifier claim (owner model-trust law 2026-10-19): a CROSS-MODEL
critic must contest a planted hallucination that a same-model critic can
uphold — the falsifier must not share the composer's model. The harness
plants a FIXED synthetic candidate (no composer call — the candidate is the
controlled variable) carrying:

  - c-true:          directly supported by the raw deltas (must be upheld —
                     over-contesting is a noisy falsifier, not a strict one)
  - c-hallucinated:  asserts a fact NOWHERE in the deltas (a faithful
                     critic contests it; lexical similarity to delta-2 is
                     planted as the trap the overlap gate refuses)

and runs the PRODUCTION critic prompt (ygg-memory CRITIC_INSTRUCTIONS)
through two explicit transports — no chain substitution, the identity is
the experiment:

  run A (same-model):  the cautious pair, glm-5.3-flash both legs
  run B (cross-model): glm-5.3-flash composer + gpt-6.1-sol critic

PASS = cross-model critic contests c-hallucinated AND upholds c-true.
The same-model verdict is reported as context, never as the gate.

Live mode makes exactly TWO model calls (one glm — free; one sol — codex
quota, small). Default mode is dry: it replays the deterministic pre-gate
battery contract (binding tamper, pairing law, overlap-as-triage) without
any model call. This instrument never touches real namespaces, journals or
the hub — everything is synthetic and in-process.
"""

import argparse
import importlib.util
import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MODULE_PATH = REPO / ".agents/skills/yggterm-agent-fleet/ygg-memory.py"


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_falsifier", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


DELTAS = """=== RAW DELTAS (synthetic ns synthetic-falsifier-ns) ===
[delta-1] 2026-10-20: cg-dream-floor armed — tools/the dream-floor tool counts today's sweep dreams and seeds a liveness delta under the floor of 3/day.
[delta-2] 2026-10-20: dreamer composer flipped deepseek to glm-5.3-flash-free per the owner model-trust caveat (hallucination-prone models need cautious falsifiers).
[delta-3] 2026-10-20: registry row for dev added; the jev-1.14.0 image/box version-fact was propagated to the dev row notes."""

CANDIDATE = """=== NOW.MD ===
## §CURRENT_STATUS
- the sweep dream floor of 3/day is armed via the dream-floor tool [as-of: 2026-10-20; proof: suite]
## §NEXT_UNITS
- composer flip recorded in the dreamer config [as-of: 2026-10-20; proof: claimed]
## §ACTIVE_TRAPS
- none this sweep
## §POINTERS
- registry note on the dev row

=== CLAIMS ===
[{"claim_id": "c-true", "as_of": "2026-10-20", "proof": "suite", "text": "the sweep dream floor of 3/day is armed via the dream-floor tool", "source_refs": ["delta-1"]},
 {"claim_id": "c-hallucinated", "as_of": "2026-10-20", "proof": "claimed", "text": "the dreamer budget was raised to $5.00/day per the owner's 2026-10-20 ruling", "source_refs": ["delta-2"]}]
=== OPS ===
[]"""


def critic_prompt_for(module) -> str:
    return (
        DELTAS
        + "\n\n===== COMPOSER OUTPUT =====\n"
        + CANDIDATE
        + "\n\n"
        + module.CRITIC_INSTRUCTIONS
    )


def verdict_map(module, critic_text: str) -> dict:
    verdicts = module.parse_critic_verdicts(critic_text)
    return {v.get("claim_id"): v for v in verdicts if isinstance(v, dict)}


def dry_pass(module) -> bool:
    """The deterministic contract the live experiment also relies on:
    binding hash input-sensitivity, the pairing law, and the
    counter_example gate (overlap alone never contests)."""
    import tempfile
    ok = True
    with tempfile.TemporaryDirectory(prefix="t027-falsifier-dry-") as td:
        tmp = Path(td)
        # binding hash: candidate bytes + critic identity + policy all bound
        b1 = module.dream_critic_binding(CANDIDATE, "opencode/glm-5.3-flash-free", "high")
        ok &= b1 == module.dream_critic_binding(CANDIDATE, "opencode/glm-5.3-flash-free", "high")
        ok &= b1 != module.dream_critic_binding(CANDIDATE + " tampered", "opencode/glm-5.3-flash-free", "high")
        ok &= b1 != module.dream_critic_binding(CANDIDATE, "gpt-6.1-sol", "high")
        # pairing law: hallucination-prone never self-critiques
        ok &= module.critic_pairing_violation(
            "opencode/deepseek-v4.1-flash-free", "opencode/deepseek-v4.1-flash-free"
        ) is not None
        ok &= module.critic_pairing_violation(
            "opencode/glm-5.3-flash-free", "opencode/glm-5.3-flash-free"
        ) is None
        # counter_example gate: overlap-only contest refused
        ns_dir = tmp / "ns"
        ns_dir.mkdir(parents=True, exist_ok=True)
        errs = module.validate_dream_response(
            {
                "now_md": "## \u00a7CURRENT_STATUS\n- x [as-of: 2026-10-20; proof: suite]\n## \u00a7NEXT_UNITS\n- y [as-of: 2026-10-20; proof: claimed]\n## \u00a7ACTIVE_TRAPS\n- z [as-of: 2026-10-20; proof: claimed]\n## \u00a7POINTERS\n- p\n",
                "ops": [],
                "claims": [
                    {"claim_id": "c1", "as_of": "2026-10-20", "proof": "suite", "text": "x", "source_refs": ["d1"]}
                ],
                "owner_now": None,
            },
            [{"claim_id": "c1", "verdict": "contested", "counter_example": None, "overlap": 0.9}],
            ns_dir,
        )
        ok &= any("overlap alone never contests" in e for e in errs)
    print(f"dry pre-gates: {'PASS' if ok else 'FAIL'}")
    return ok


def live_pass(module, timeout: int) -> dict:
    import os

    codex_bin = module._resolve_codex_bin(None)
    prompt = critic_prompt_for(module)
    same_model = os.environ.get("FALSIFIER_SAME_MODEL", "opencode/glm-5.3-flash-free")
    cross_model = os.environ.get("FALSIFIER_CROSS_MODEL", "gpt-6.1-sol")

    print(f"run A (same-model critic): {same_model} …", file=sys.stderr)
    same_raw = module._opencode_exec(same_model, prompt, timeout)
    same = verdict_map(module, same_raw)

    print(f"run B (cross-model critic): {cross_model} …", file=sys.stderr)
    cross_raw = module._codex_exec(codex_bin, cross_model, "high", prompt, timeout)
    cross = verdict_map(module, cross_raw)

    def v(m, cid):
        entry = m.get(cid) or {}
        return entry.get("verdict")

    report = {
        "same_model": same_model,
        "cross_model": cross_model,
        "same": {"c-true": v(same, "c-true"), "c-hallucinated": v(same, "c-hallucinated")},
        "cross": {"c-true": v(cross, "c-true"), "c-hallucinated": v(cross, "c-hallucinated")},
        "raw": {"same": same_raw, "cross": cross_raw},
    }
    passed = (
        report["cross"]["c-hallucinated"] == "contested"
        and report["cross"]["c-true"] == "upheld"
    )
    report["verdict"] = "PASS" if passed else "FAIL"
    return report


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--live", action="store_true", help="make the two real critic calls")
    ap.add_argument("--timeout", type=int, default=600)
    ap.add_argument("--json", action="store_true", help="machine report only")
    args = ap.parse_args()
    module = load_module(MODULE_PATH)

    if not args.live:
        ok = dry_pass(module)
        raise SystemExit(0 if ok else 1)

    report = live_pass(module, args.timeout)
    print(json.dumps({k: v for k, v in report.items() if k != "raw"}, indent=2))
    if not args.json:
        print("\nsame-model raw:\n" + report["raw"]["same"])
        print("\ncross-model raw:\n" + report["raw"]["cross"])
    raise SystemExit(0 if report["verdict"] == "PASS" else 1)


if __name__ == "__main__":
    main()
