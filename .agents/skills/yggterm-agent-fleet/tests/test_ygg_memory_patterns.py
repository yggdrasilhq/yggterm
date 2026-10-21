#!/usr/bin/env python3
"""[slice 8] pattern learning: evidence-linking, repeated-copy folding,
authority discipline, holdout-gated promotion (gap 4 falsifier in code).

    python3 tests/test_ygg_memory_patterns.py [path-to-ygg-memory.py]

Gap 4 (lores 2026-10-16-yggterm-memory-post-build-gaps-sol): proposals carry
supporting episodes, counterexamples, scope and uncertainty; repetition
copied across summaries is NEVER independent evidence; observed practice is
distinguished from owner instruction; promotion needs the measured holdout
gate ref and stays owner-owned.
"""

import importlib.util
import json
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_TARGET = HERE.parent / "ygg-memory.py"


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_patterns_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


passed = failed = 0


def check(name, condition, detail=""):
    global passed, failed
    if condition:
        passed += 1
        print(f"  ok  {name}")
    else:
        failed += 1
        print(f" FAIL {name} {detail}")


def expect_system_exit(fn, needle=""):
    try:
        fn()
    except SystemExit as exit_exc:
        text = str(exit_exc)
        return (not needle) or (needle in text)
    except Exception as exc:  # noqa: BLE001
        return False, f"wrong exception type: {type(exc).__name__}: {exc}"
    return False, "no refusal"


def main(argv):
    target = Path(argv[1]) if len(argv) > 1 else DEFAULT_TARGET
    ym = load_module(target)
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        ns = ym.detect_namespace(override="nsA")  # store key, CLI-normalized (-nsA)

        # ── admission guards (fail-closed) ──
        episodes = [{"kind": "case", "ref": "9ab1c2ff", "eid": "1111111111111111",
                     "as_of": ym.budget_today()}]

        check("unstated authority refuses", expect_system_exit(
            lambda: ym.pattern_propose(root, ns, "s", "sc", "gut-feeling",
                                       uncertainty="low", uncertainty_reason="r",
                                       episodes=episodes),
            "authority must be one of"))
        check("owner-instruction needs owner ref", expect_system_exit(
            lambda: ym.pattern_propose(root, ns, "s", "sc", "owner-instruction",
                                       uncertainty="low", uncertainty_reason="r",
                                       episodes=episodes),
            "requires --owner-source-ref"))
        check("no episodes refuses (opinion, not pattern)", expect_system_exit(
            lambda: ym.pattern_propose(root, ns, "s", "sc", "observed-practice",
                                       uncertainty="low", uncertainty_reason="r"),
            "at least one supporting episode"))
        check("unsearched counterexamples refuse", expect_system_exit(
            lambda: ym.pattern_propose(root, ns, "s", "sc", "observed-practice",
                                       uncertainty="low", uncertainty_reason="r",
                                       episodes=episodes),
            "counterexample search must be honest"))
        check("empty scope refuses", expect_system_exit(
            lambda: ym.pattern_propose(root, ns, "s", "", "observed-practice",
                                       uncertainty="low", uncertainty_reason="r",
                                       episodes=episodes,
                                       no_counterexample_found="searched all"),
            "--statement and --scope"))

        created = ym.pattern_propose(
            root, ns, "yggterm-* doors own -home-pi-gh-yggterm",
            "hub-door routing", "observed-practice",
            uncertainty="medium", uncertainty_reason="small n on the hub family",
            episodes=episodes, no_counterexample_found="grepped all dev episodes",
        )
        check("created returns id", created["action"] == "created" and created["id"].startswith("pat-"))
        check("distinct_sources counted", created["distinct_sources"] == 1)

        # ── repetition folding: same episode identity re-proposed = dedup ──
        again = ym.pattern_propose(
            root, ns, "yggterm-* doors own -home-pi-gh-yggterm",
            "hub-door routing", "observed-practice",
            uncertainty="medium", uncertainty_reason="small n on the hub family",
            episodes=episodes, no_counterexample_found="grepped all dev episodes",
        )
        check("identical rerun deduplicates (never re-counts)", again["action"] == "deduplicated")
        check("dedupe keeps sources at 1", again["distinct_sources"] == 1)

        # a NEW distinct episode strengthens; a copied variant folds by eid
        fresh = dict(episodes[0]); fresh["ref"] = "f10c773d"; fresh["eid"] = "2222222222222222"
        strengthened = ym.pattern_propose(
            root, ns, "yggterm-* doors own -home-pi-gh-yggterm",
            "hub-door routing", "observed-practice",
            uncertainty="medium", uncertainty_reason="small n on the hub family",
            episodes=[fresh], no_counterexample_found="grepped all dev episodes",
        )
        check("new distinct episode strengthens", strengthened["action"] == "strengthened"
              and strengthened["distinct_sources"] == 2)
        copy = dict(episodes[0])  # same content, different ref-wrap → folds by eid
        copy["ref"] = "summary-copied"
        folded = ym.pattern_propose(
            root, ns, "yggterm-* doors own -home-pi-gh-yggterm",
            "hub-door routing", "observed-practice",
            uncertainty="medium", uncertainty_reason="small n on the hub family",
            episodes=[copy], no_counterexample_found="grepped all dev episodes",
        )
        check("repetition copied across summaries folds", folded["action"] == "deduplicated"
              and folded["distinct_sources"] == 2)

        # ── the promotion gate (falsifier in code) ──
        pid = created["id"]
        check("promoted refuses outright (owner-only)", expect_system_exit(
            lambda: ym.pattern_set_status(root, ns, pid, "promoted"),
            "owner-only"))
        check("promotion-eligible without a gate ref refuses", expect_system_exit(
            lambda: ym.pattern_set_status(root, ns, pid, "promotion-eligible"),
            "measured holdout gate ref"))
        ok_gate = ym.pattern_set_status(root, ns, pid, "promotion-eligible",
                                        ref="heldout 2026-10-21: +Δ acc measured, 0 invented")
        check("promotion-eligible with gate ref lands", ok_gate["status"] == "promotion-eligible")
        data = ym.load_patterns(root, ns)
        row = [p for p in data["patterns"] if p["id"] == pid][0]
        check("gate ref recorded", row.get("status_ref", "").startswith("heldout"))

        # ── seat-level transitions + list view ──
        ym.pattern_set_status(root, ns, pid, "rejected")
        ns_store = ym.detect_namespace(override="nsA")  # CLI normalization (-nsA)
        out = json.loads(__import__("subprocess").run(
            [sys.executable, str(target), "patterns", "--root", str(root),
             f"--ns={ns_store}", "--list", "--json"],
            capture_output=True, text=True).stdout)
        rows = [p for p in out["patterns"] if p["id"] == pid]
        check("rejected row still listed by default", len(rows) == 1
              and rows[0]["status"] == "rejected")

    print(f"\npassed {passed} / failed {failed}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
