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


    print("[3] namespace-qualified scope matching (fence fix 2026-10-16)")
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        write_journal(root, [{"seq": 10, "ts": time.time() - 3600, "ns": "ns-x", "file": "a.md", "summary": "prepare-era write"}])
        write_journal(root, [{"seq": 11, "ts": time.time() - 3500, "ns": "_global", "file": "advisors-doctrine.md", "summary": "native mirror sync touches the _global namesake"}])
        fence = module.dream_fence_violation(root, "ns-x", 10, ["advisors-doctrine.md"])
        check(fence == "", "a _global record with the same bare filename never fences another ns's scoped dream (the 33180 false-block class)")
        write_journal(root, [{"seq": 12, "ts": time.time() - 60, "ns": "ns-x", "file": "advisors-doctrine.md", "summary": "same-ns concurrent write to a composed door"}])
        fence = module.dream_fence_violation(root, "ns-x", 10, ["advisors-doctrine.md"])
        check(bool(fence) and "composed" in fence, "a same-ns write to a composed door still fences")
        write_journal(root, [{"seq": 13, "ts": time.time() - 30, "ns": "ns-x", "summary": "record with no file attribution"}])
        fence = module.dream_fence_violation(root, "ns-x", 12, ["advisors-doctrine.md"])
        check(bool(fence), "an unattributed-file record still refuses (conservative)")
        write_journal(root, [{"seq": 14, "ts": time.time() - 20, "ns": "ns-x", "file": "a.md", "summary": "record with no ns attribution"}])
        fence = module.dream_fence_violation(root, "ns-x", 12, ["a.md"])
        # strip the ns on a copy to simulate a legacy unattributed record
        import json as _json
        jr = root / "journal.jsonl"
        rows = [_json.loads(l) for l in open(jr)]
        rows[-1].pop("ns", None)
        jr.write_text("".join(_json.dumps(r) + "\n" for r in rows))
        fence = module.dream_fence_violation(root, "ns-x", 12, ["a.md"])
        check(bool(fence), "an unattributed-ns record with a matching filename still refuses (conservative)")
        write_journal(root, [{"seq": 15, "ts": time.time() - 10, "ns": "_global", "file": "owner-now.md", "summary": "concurrent owner-now write"}])
        fence = module.dream_fence_violation(root, "_global", 14, ["owner-now.md"])
        check(bool(fence) and "composed" in fence, "a _global dream still fences on _global composed doors")

    if failures:
        print(f"\n{len(failures)} FAILURES")
        return 1
    print("\nall fence locks green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
