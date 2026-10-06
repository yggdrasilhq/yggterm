#!/usr/bin/env python3
"""[mem-dream slice 1.2] the two dream fences: quiescence (gate 3) + read-version fencing (gate 4).

    python3 tests/test_dream_fences.py [path-to-ygg-memory.py]

⛔ THE HOLES THIS PINS (gemini round-3 consult, node
2026-10-06-yggterm-memory-dreaming-round3-fault-tolerance.md): the dreamer
is a slow LLM writer inside a 24/7 multi-agent file mesh. (1) Without
QUIESCENCE it competes with live sittings for the write plane mid-pack-up.
(2) Without READ-VERSION FENCING, a 30-60s composer delay can apply a
rollup whose factual foundation was invalidated by a journal write that
landed while it was thinking. Both fences REFUSE loudly; dream-origin
records never fence or block the dream itself.
"""

import importlib.util
import json
import sys
import tempfile
import time
from pathlib import Path


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_under_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def write_journal(root: Path, records: list) -> None:
    journal = root / "journal.jsonl"
    journal.parent.mkdir(parents=True, exist_ok=True)
    with open(journal, "a", encoding="utf-8") as handle:
        for record in records:
            handle.write(json.dumps(record) + "\n")


def main() -> int:
    default = Path(__file__).resolve().parent.parent / "ygg-memory.py"
    module = load_module(Path(sys.argv[1]) if len(sys.argv) > 1 else default)
    failures = []

    def check(condition, message):
        if condition:
            print(f"  ok: {message}")
        else:
            failures.append(message)
            print(f"  FAIL: {message}")

    print("[1] quiescence (gate 3)")
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        write_journal(root, [{"seq": 5, "ts": time.time() - 30, "ns": "ns-x", "summary": "sitting pack-up"}])
        violation = module.dream_quiescence_violation(root, "ns-x")
        check(bool(violation) and "ACTIVE" in violation, "a write 30s ago defers the dream")
        root = Path(tempfile.mkdtemp())
        write_journal(root, [{"seq": 6, "ts": time.time() - 1800, "ns": "ns-x", "summary": "old sitting"}])
        write_journal(root, [{"seq": 7, "ts": time.time() - 60, "ns": "ns-x", "summary": "dream rollup today"}])
        write_journal(root, [{"seq": 8, "ts": time.time() - 90, "ns": "_global", "summary": "owner-now update today"}])
        violation = module.dream_quiescence_violation(root, "ns-x")
        check(violation == "", "dream/owner-now records never block the dream; old writes do not")
        write_journal(root, [{"seq": 9, "ts": time.time() - 60, "ns": "_global", "summary": "owner edits a global door"}])
        violation = module.dream_quiescence_violation(root, "ns-x")
        check(bool(violation), "a live _global write defers the dream (the dreamer writes _global too)")
        check(module.dream_quiescence_violation(Path(tmp) / "empty", "ns-x") == "", "an empty journal is quiescent")

    print("[2] read-version fencing (gate 4)")
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        write_journal(root, [{"seq": 10, "ts": time.time() - 3600, "ns": "ns-x", "summary": "prepare-era write"}])
        fence = module.dream_fence_violation(root, "ns-x", 10)
        check(fence == "", "no records after captured_seq — apply proceeds")
        write_journal(root, [{"seq": 11, "ts": time.time() - 3500, "ns": "ns-x", "summary": "dream op: append"}])
        fence = module.dream_fence_violation(root, "ns-x", 10)
        check(fence == "", "a dream-origin record after prepare never fences the dream")
        write_journal(root, [{"seq": 12, "ts": time.time() - 60, "ns": "other-ns", "summary": "unrelated ns write"}])
        fence = module.dream_fence_violation(root, "ns-x", 10)
        check(fence == "", "another namespace's write never fences this ns's dream")
        write_journal(root, [{"seq": 13, "ts": time.time() - 30, "ns": "ns-x", "summary": "mid-composer sitting write"}])
        fence = module.dream_fence_violation(root, "ns-x", 10)
        check(bool(fence) and "re-prepare" in fence, "a live write during the composer window refuses the apply")

    if failures:
        print(f"\n{len(failures)} FAILURES")
        return 1
    print("\nall fence locks green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
