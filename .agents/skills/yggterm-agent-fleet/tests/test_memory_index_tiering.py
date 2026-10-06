#!/usr/bin/env python3
"""[mem-dream slice 1] tiered index + shelve-archives: classifier, index split, verb semantics.

    python3 tests/test_memory_index_tiering.py [path-to-ygg-memory.py]

⛔ THE HOLE THIS PINS, measured 2026-10-06: the zcode MEMORY.md index was
rebuilt from EVERY *.md under the memory tree — 906 lines / 176.7KB, past
the harness loader's partial-load cut — so index lines past the cut never
loaded and their doors were unreachable THROUGH THE INDEX while remaining
fully present on disk. The tiering law under test: shelved docs (archive/
path, explicit sitting-archive/archive frontmatter type, or the
sitting/session/wave-N/delivery/-history/dated name classes) leave the
injected index for ARCHIVE-INDEX.md; living doors keep the byte-identical
line format; the shelve verb is dry-run by default, skips on collision,
and is idempotent.
"""

import importlib.util
import sys
import tempfile
from pathlib import Path


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_under_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


DOOR = """---
name: {name}
description: a door
metadata:
  type: {kind}
---

body of {name}
"""


def write_door(directory: Path, name: str, kind: str = "project") -> None:
    (directory / f"{name}.md").write_text(DOOR.format(name=name, kind=kind), encoding="utf-8")


def build_store(home: Path) -> Path:
    memory_dir = home / "projects" / "default-x" / "memory"
    memory_dir.mkdir(parents=True)
    write_door(memory_dir, "living-door")
    write_door(memory_dir, "campaign-x-sitting1-landed-2026-10-06")  # dated + sitting
    write_door(memory_dir, "topic-history")  # -history class
    write_door(memory_dir, "explicit-shelved", kind="sitting-archive")
    archive = memory_dir / "archive"
    archive.mkdir()
    write_door(archive, "already-archived")
    return memory_dir


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

    # 1 — the classifier: one rule set for index and verb alike.
    print("[1] classifier")
    check(module._is_shelved_doc(("archive",), "anything.md", ""), "archive/ path shelves")
    check(module._is_shelved_doc((), "x.md", "sitting-archive"), "explicit type shelves")
    check(module._is_shelved_doc((), "cli-integration-session-2-late.md", ""), "session name shelves")
    check(module._is_shelved_doc((), "practice-l2-fi-wave-0091.md", ""), "wave-N name shelves")
    check(module._is_shelved_doc((), "delivery-2026-09-19.md", ""), "delivery + dated name shelves")
    check(module._is_shelved_doc((), "campaign-mackvm-history.md", ""), "-history name shelves")
    check(
        not module._is_shelved_doc((), "campaign-yggterm.md", "project"),
        "a living campaign door stays living",
    )
    check(
        not module._is_shelved_doc((), "memory-dreaming-design.md", "project"),
        "an undated design door stays living",
    )
    check(
        not module._is_shelved_doc(("pinned", "yggterm"), "campaign-yggterm.md", "project"),
        "delivered hub doors stay living",
    )

    # 2 — the tiered index: living doors only in MEMORY.md, shelved in ARCHIVE-INDEX.md.
    print("[2] tiered index rebuild")
    with tempfile.TemporaryDirectory() as tmp:
        home = Path(tmp)
        adapter = module.ZcodeMemoryAdapter(home)
        memory_dir = build_store(home)
        adapter._rebuild_index(memory_dir)
        index = (memory_dir / "MEMORY.md").read_text(encoding="utf-8")
        archive_index = (memory_dir / "ARCHIVE-INDEX.md").read_text(encoding="utf-8")
        check("living-door" in index, "MEMORY.md lists the living door")
        check(
            "[living-door](living-door.md) — a door" in index,
            "living line format is byte-identical to the legacy format",
        )
        check("sitting1-landed" not in index, "MEMORY.md drops the dated sitting file")
        check("topic-history" not in index, "MEMORY.md drops the -history file")
        check("explicit-shelved" not in index, "MEMORY.md drops the explicit-type file")
        check("already-archived" not in index, "MEMORY.md drops archive/ residents")
        check("ARCHIVE-INDEX" not in index, "MEMORY.md never lists the archive index itself")
        check(
            all(
                marker in archive_index
                for marker in ("sitting1-landed", "topic-history", "explicit-shelved", "already-archived")
            ),
            "ARCHIVE-INDEX.md lists every shelved doc",
        )
        check("living-door" not in archive_index, "ARCHIVE-INDEX.md excludes living doors")
        # A second rebuild is stable (the archive index must not index itself).
        adapter._rebuild_index(memory_dir)
        check(
            (memory_dir / "ARCHIVE-INDEX.md").read_text(encoding="utf-8") == archive_index,
            "rebuild is idempotent — ARCHIVE-INDEX.md never lists itself",
        )

    # 3 — the shelve verb: dry-run moves nothing; apply moves exactly the
    #     top-level shelved class; skip-on-collision; idempotent.
    print("[3] shelve-archives semantics")
    with tempfile.TemporaryDirectory() as tmp:
        home = Path(tmp)
        adapter = module.ZcodeMemoryAdapter(home)
        memory_dir = build_store(home)
        candidates, moved = module.shelve_legacy_archives(adapter, memory_dir, apply=False)
        check(sorted(doc.name for doc in candidates) == [
            "campaign-x-sitting1-landed-2026-10-06.md",
            "explicit-shelved.md",
            "topic-history.md",
        ], "dry-run finds exactly the three top-level shelved docs")
        check(moved == 0 and (memory_dir / "topic-history.md").is_file(), "dry-run moves nothing")
        candidates, moved = module.shelve_legacy_archives(adapter, memory_dir, apply=True)
        check(moved == 4, "apply moves all three candidates + migrates the legacy in-tree archive resident")
        archive_dir = memory_dir.parent / "memory-archive"
        check(
            (archive_dir / "topic-history.md").is_file()
            and (archive_dir / "explicit-shelved.md").is_file()
            and (archive_dir / "campaign-x-sitting1-landed-2026-10-06.md").is_file(),
            "moved docs land in ../memory-archive/ (OUTSIDE the scanned tree) under their own names",
        )
        check(
            not (memory_dir / "archive").exists(),
            "legacy in-tree archive/ dir is migrated and removed in the same apply",
        )
        adapter._rebuild_index(memory_dir)
        index = (memory_dir / "MEMORY.md").read_text(encoding="utf-8")
        check("living-door" in index and "topic-history" not in index, "index reflects the shelve")
        candidates, moved = module.shelve_legacy_archives(adapter, memory_dir, apply=True)
        check(
            not candidates and moved == 0 and not (memory_dir / "archive").is_dir(),
            "second apply is a no-op once candidates and legacy are gone",
        )
        check(
            (archive_dir / "already-archived.md").is_file(),
            "legacy in-tree archive resident migrated into memory-archive/",
        )
        adapter._rebuild_index(memory_dir)
        archive_index = (memory_dir / "ARCHIVE-INDEX.md").read_text(encoding="utf-8")
        check(
            "../memory-archive/" in archive_index and "4 docs" in archive_index,
            "ARCHIVE-INDEX.md points at the external archive with its doc count",
        )
        candidates, moved = module.shelve_legacy_archives(adapter, memory_dir, apply=True)
        check(not candidates and moved == 0, "third apply is a no-op (idempotent)")

    if failures:
        print(f"\n{len(failures)} FAILURES")
        return 1
    print("\nall tiering locks green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
