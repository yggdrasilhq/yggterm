#!/usr/bin/env python3
"""[T-002 m2] the dreamer tick's fleet-task-ledger pickup.

    python3 tests/test_task_pickup.py [path-to-ygg-memory.py]

plan.py (tools/task-ledger/) is the tokenomics SPEC: score = value/weight
(big 4 / moderate 2 / small 1), deps-not-ready held out, OWNER-GATED never
auto. The owner correction 2026-10-17 binds the harness rule: codex takes
moderate-to-SMALL ONLY — a big task never routes to codex whole. The tick's
pickup is env-gated (YGG_DREAM_TASK_PICKUP=1): the owner's 3-task GLM idle
allowance is HIS quota — a dream tick never claims a task silently. Claims
book through the publish-safe path; a task no longer open in the text being
claimed refuses (another seat took it).
"""

import importlib.util
import json
import os
import sys
import tempfile
from pathlib import Path

MODULE_PATH = Path(__file__).resolve().parent.parent / "ygg-memory.py"


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_under_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


SYNTHETIC_LEDGER = """
### [T-101] low value small codex
- size: small ; value: 1 ; est: x
- route: codex ; deps: none
- status: open

### [T-102] high value big glm
- size: big ; value: 5 ; est: x
- route: glm-idle ; deps: none
- status: open

### [T-103] high value moderate glm
- size: moderate ; value: 5 ; est: x
- route: glm-idle ; deps: none
- status: open

### [T-104] gated panel
- size: small ; value: 4 ; est: x
- route: OWNER-GATED ; deps: none
- status: open (gated)

### [T-105] held for deps
- size: moderate ; value: 5 ; est: x
- route: glm-idle ; deps: some blocking thing
- status: open

### [T-106] second moderate any
- size: moderate ; value: 4 ; est: x
- route: any ; deps: none
- status: open

### [T-107] already claimed
- size: moderate ; value: 5 ; est: x
- route: glm-idle ; deps: none
- status: claimed(zcode, 2026-10-17) — taken

### [T-108] blocked on owner
- size: moderate ; value: 5 ; est: x
- route: glm-idle ; deps: none
- status: blocked(owner-scan-pending)

### [T-109] codex-routed big (never whole)
- size: big ; value: 5 ; est: x
- route: codex ; deps: none
- status: open

## §BOOK — one-liners of finished work
"""


def test_parse_and_score(module, failures):
    tasks = module.ledger_tasks(SYNTHETIC_LEDGER)
    if len(tasks) != 9:
        failures.append(f"pickup: parse count wrong: {len(tasks)}")
        return
    by_id = {t["id"]: t for t in tasks}
    score, size = module.ledger_task_score(by_id["T-102"])
    if (score, size) != (1.25, "big"):
        failures.append(f"pickup: T-102 score/size wrong: {score} {size}")
    score, size = module.ledger_task_score(by_id["T-103"])
    if (score, size) != (2.5, "moderate"):
        failures.append(f"pickup: T-103 score/size wrong: {score} {size}")


def test_pick_respects_routing_law(module, failures):
    tasks = module.ledger_tasks(SYNTHETIC_LEDGER)
    glm_pick = module.ledger_pick(tasks, "zcode")
    if glm_pick is None or glm_pick["id"] != "T-103":
        failures.append(f"pickup: glm should take T-103 (score 2.5, the top ready): {glm_pick and glm_pick['id']}")
    # T-102 scores 1.25 < 2.5; the top-scoring READY task wins regardless of size
    codex_pick = module.ledger_pick(tasks, "codex")
    # codex cannot take T-102 (big), T-103/T-107 are glm-routed, T-104 gated,
    # T-105 deps-held, T-108 blocked, T-109 is codex-routed BUT big -> the
    # top codex-legal ready task is T-106 (moderate, any-route, score 2.0)
    if codex_pick is None or codex_pick["id"] != "T-106":
        failures.append(f"pickup: codex should take T-106, not big/not glm-routed: {codex_pick and codex_pick['id']}")
    # codex NEVER takes a big even when it is the only ready task
    big_only = [t for t in tasks if t["id"] in ("T-102", "T-109")]
    if module.ledger_pick(big_only, "codex") is not None:
        failures.append("pickup: codex took a BIG task — routing law violated")


def test_claim_transform_and_refusal(module, failures):
    CLAIMED_LINE = "- status: claimed(zcode, 2026-10-17) — taken by the dreamer pickup; execution in flight"
    claimed = module.ledger_claim_text(SYNTHETIC_LEDGER, "T-103", "zcode", "2026-10-17")
    if CLAIMED_LINE not in claimed:
        failures.append("pickup: claim transform did not book the status line")
    if CLAIMED_LINE in SYNTHETIC_LEDGER:
        failures.append("pickup: claim mutated its input (pure transform violated)")
    # already-claimed refuses
    try:
        module.ledger_claim_text(claimed, "T-103", "codex", "2026-10-17")
        failures.append("pickup: claiming an already-claimed task did not refuse")
    except SystemExit:
        pass
    # unknown id refuses
    try:
        module.ledger_claim_text(SYNTHETIC_LEDGER, "T-999", "zcode", "2026-10-17")
        failures.append("pickup: unknown task id did not refuse")
    except SystemExit:
        pass


def test_pickup_cmd_dry_run(module, failures):
    """--dry-run drives the full pick path against a tmp root WITHOUT
    publishing (no subprocess, no hub touch)."""
    import contextlib
    import io

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        ns_dir = module.get_namespace_dir(root, module.LEDGER_NS)
        ns_dir.mkdir(parents=True, exist_ok=True)
        (ns_dir / module.LEDGER_DOOR).write_text(SYNTHETIC_LEDGER, encoding="utf-8")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            module.cmd_task_pickup(
                _PickupArgs(root=str(root), harness="zcode", dry_run=True, json=True)
            )
        result = json.loads(buf.getvalue())
        if result.get("picked") != "T-103":
            failures.append(f"pickup: dry-run picked {result.get('picked')} not T-103")
        if result.get("harness") != "zcode":
            failures.append(f"pickup: dry-run harness wrong: {result.get('harness')}")
        if "dry_run" not in result or "preview" not in result:
            failures.append(f"pickup: dry-run result missing preview: {result.keys()}")
        if "- status: claimed(zcode," not in result.get("preview", ""):
            failures.append("pickup: dry-run preview missing the claim transform")
        # the door file itself stays untouched by a dry run
        door = (ns_dir / module.LEDGER_DOOR).read_text(encoding="utf-8")
        if "claimed(zcode, 2026-10-17) — taken by the dreamer pickup" in door:
            failures.append("pickup: DRY RUN published a claim — hub touched")


class _PickupArgs:
    def __init__(self, **kw):
        self.subcommand = "task-pickup"
        self.root = None
        self.harness = None
        self.dry_run = False
        self.json = True
        self.__dict__.update(kw)


def main() -> int:
    module = load_module(Path(sys.argv[1]) if len(sys.argv) > 1 else MODULE_PATH)
    failures: list[str] = []
    test_parse_and_score(module, failures)
    test_pick_respects_routing_law(module, failures)
    test_claim_transform_and_refusal(module, failures)
    test_pickup_cmd_dry_run(module, failures)
    if failures:
        print(f"task-pickup tests: {len(failures)} FAILURE(S)")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("task-pickup tests: all green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
