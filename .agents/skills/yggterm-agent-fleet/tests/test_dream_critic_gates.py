#!/usr/bin/env python3
"""[T-027 sol pre-gates] cross-model critic gates: the verdict→candidate
binding hash, contested-blocks-dependent-ops, claim-ID rollup statements,
the critic roster, and the planted-hallucination falsifier wiring.

    python3 tests/test_dream_critic_gates.py [path-to-ygg-memory.py]

⛔ THE HOLES THIS PINS (sol verdicts 2026-10-19, owner model-trust law):
a critic verdict set that is not BOUND to the exact candidate bytes can be
swapped, stale, or from another namespace and still validate claims it
never saw; a contested claim must not silently carry apply ops that depend
on it; rollup statements without claim IDs cannot be traced to their
falsifier; and the cross-model falsifier must be reachable without
re-deriving the roster per seat.
"""

import importlib.util
import json
import re
import sys
import tempfile
from pathlib import Path

MODULE_PATH = Path(__file__).resolve().parent.parent / "ygg-memory.py"

NOW_MD = """## §CURRENT_STATUS
- alpha claim landed [claim: c1] [as-of: 2026-10-20; proof: suite]
- beta claim observed [claim: c2] [as-of: 2026-10-20; proof: measured-live]

## §NEXT_UNITS
- gamma queued [claim: c3] [as-of: 2026-10-20; proof: claimed]

## §ACTIVE_TRAPS
- delta trap watched [claim: c4] [as-of: 2026-10-20; proof: claimed]

## §POINTERS
- a pointer door
"""

CLAIMS = [
    {"claim_id": "c1", "text": "alpha claim landed", "as_of": "2026-10-20", "proof": "suite", "source_refs": ["seq#1/door-a.md"]},
    {"claim_id": "c2", "text": "beta claim observed", "as_of": "2026-10-20", "proof": "measured-live", "source_refs": ["seq#1/door-a.md"]},
    {"claim_id": "c3", "text": "gamma queued", "as_of": "2026-10-20", "proof": "claimed", "source_refs": ["seq#1/door-a.md"]},
    {"claim_id": "c4", "text": "delta trap watched", "as_of": "2026-10-20", "proof": "claimed", "source_refs": ["seq#1/door-a.md"]},
]

OP_APPEND = {"op": "append-section", "file": "door-a.md", "section": "§GOTCHAS",
             "entry": "- gamma queued for the next slice", "reason": "gamma queued depends on c3"}


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_gates_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def parsed_response(ops=None):
    return {"now_md": NOW_MD, "ops": ops or [], "claims": json.loads(json.dumps(CLAIMS)), "owner_now": None}


def verdicts(*overrides):
    base = {c["claim_id"]: {"claim_id": c["claim_id"], "verdict": "upheld", "counter_example": None} for c in CLAIMS}
    base.update({o[0]: {"claim_id": o[0], "verdict": o[1], "counter_example": None} for o in overrides})
    return list(base.values())


def build_ns(module, root: Path):
    ns = "-t-gates"
    module.validate_namespace(ns)
    ns_dir = module.get_namespace_dir(root, ns, create=True)
    (ns_dir / "door-a.md").write_text("# door a\n\n§GOTCHAS marker below\n\n§GOTCHAS\n", encoding="utf-8")
    return ns, ns_dir


def main() -> int:
    failures: list[str] = []
    module = load_module(MODULE_PATH)

    # --- binding hash: deterministic, input-sensitive -----------------------
    b1 = module.dream_critic_binding("resp", "m1", "high")
    if b1 != module.dream_critic_binding("resp", "m1", "high"):
        failures.append("binding: not deterministic")
    if b1 == module.dream_critic_binding("resp2", "m1", "high"):
        failures.append("binding: response bytes not bound")
    if b1 == module.dream_critic_binding("resp", "m2", "high"):
        failures.append("binding: critic model not bound")
    if b1 == module.dream_critic_binding("resp", "m1", "low"):
        failures.append("binding: critic effort not bound")
    if module.DREAM_CRITIC_POLICY != "t027-gates-1":
        failures.append("binding: policy version drifted from the tested value")

    # --- contested blocks dependent ops (strict only) -----------------------
    with tempfile.TemporaryDirectory() as tmp:
        _, ns_dir = build_ns(module, Path(tmp))
        base = parsed_response()
        vs = verdicts(("c3", "contested"))

        # strict + an op whose reason cites the contested claim id: blocked
        strict_resp = parsed_response(ops=[dict(OP_APPEND)])
        errs = module.validate_dream_response(strict_resp, vs, ns_dir, strict=True)
        if not any("depends on contested claim c3" in e for e in errs):
            failures.append(f"contested-dependents: op citing c3 not blocked: {errs}")
        # non-strict (legacy): same shape stays valid
        errs = module.validate_dream_response(parsed_response(ops=[dict(OP_APPEND)]), vs, ns_dir)
        if any("depends on contested claim" in e for e in errs):
            failures.append("contested-dependents: legacy path wrongly blocked")
        # an op with no dependency on the contested claim rides fine
        clean_op = {"op": "append-section", "file": "door-a.md", "section": "§GOTCHAS",
                    "entry": "- unrelated bullet", "reason": "unrelated to any claim"}
        errs = module.validate_dream_response(parsed_response(ops=[clean_op]), vs, ns_dir, strict=True)
        if any("depends on contested claim" in e for e in errs):
            failures.append("contested-dependents: clean op wrongly blocked")

    # --- claim-ID on every rollup statement (strict only) -------------------
    with tempfile.TemporaryDirectory() as tmp:
        _, ns_dir = build_ns(module, Path(tmp))
        stripped = parsed_response()
        stripped["now_md"] = NOW_MD.replace("[claim: c1] ", "")
        errs = module.validate_dream_response(stripped, verdicts(), ns_dir, strict=True)
        if not any("missing [claim: id]" in e for e in errs):
            failures.append(f"claim-id: strict path did not require the id: {errs}")
        errs = module.validate_dream_response(parsed_response(), verdicts(), ns_dir)
        if any("missing [claim: id]" in e for e in errs):
            failures.append("claim-id: legacy path wrongly required the id")
        unknown = parsed_response()
        unknown["now_md"] = NOW_MD.replace("[claim: c1]", "[claim: zzz]")
        errs = module.validate_dream_response(unknown, verdicts(), ns_dir, strict=True)
        if not any("unknown claim 'zzz'" in e for e in errs):
            failures.append("claim-id: unknown claim reference accepted")

    # --- contested claim id cited without demotion (strict) -----------------
    with tempfile.TemporaryDirectory() as tmp:
        _, ns_dir = build_ns(module, Path(tmp))
        demoted = parsed_response()
        demoted["now_md"] = NOW_MD.replace(
            "- gamma queued [claim: c3]",
            "- gamma queued [claim: c3] [CONTESTED: counter-example]")
        vs = verdicts(("c3", "contested"))
        errs = module.validate_dream_response(demoted, vs, ns_dir, strict=True)
        if any("cited without demotion" in e for e in errs):
            failures.append("demotion: visibly demoted contested bullet rejected")
        errs = module.validate_dream_response(parsed_response(), vs, ns_dir, strict=True)
        if not any("cited without demotion" in e for e in errs):
            failures.append("demotion: contested citation without demotion accepted")

    # --- roster: file loading, override semantics, fallback -----------------
    # The roster names PRIVATE namespaces, so the shipped default is empty
    # and the mechanism is tested with neutral names through the file.
    if module.DEFAULT_CRITIC_ROSTER != {}:
        failures.append("roster: public default must stay empty (private ns names)")
    import os as _os
    cfg = Path.home() / ".yggterm" / "config" / "dream"
    probe = cfg / "critic-roster.json"
    saved = probe.read_text(encoding="utf-8") if probe.is_file() else None
    try:
        cfg.mkdir(parents=True, exist_ok=True)
        probe.write_text(json.dumps({
            "example-ns-a": {"critic_model": "test-critic", "critic_effort": "low"},
            "example-ns-b": None,
        }), encoding="utf-8")
        roster = module.load_critic_roster()
        if roster.get("example-ns-a", {}).get("critic_model") != "test-critic":
            failures.append("roster: file override not applied")
        if roster.get("example-ns-a", {}).get("critic_effort") != "low":
            failures.append("roster: critic_effort override not applied")
        if "example-ns-b" in roster:
            failures.append("roster: null entry not removed")
    finally:
        if saved is None:
            probe.unlink(missing_ok=True)
        else:
            probe.write_text(saved, encoding="utf-8")
    probe.unlink(missing_ok=True)
    if module.load_critic_roster() != {}:
        failures.append("roster: absent file must yield the empty default")

    # --- critic instructions carry the triage rule --------------------------
    if "TRIAGE" not in module.CRITIC_INSTRUCTIONS or "never a verdict" not in module.CRITIC_INSTRUCTIONS:
        failures.append("critic: lexical/citation triage rule missing from instructions")

    if failures:
        print("dream-critic-gates tests: FAILURES")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("dream-critic-gates tests: all green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
