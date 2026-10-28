#!/usr/bin/env python3
"""Native sync must not reread/rewrite a whole harness checkpoint per door.

Run with an optional runner path to falsify an older implementation.
All data is synthetic; TMPDIR can point at disk-backed agent scratch.
"""
import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

RUNNER = Path(sys.argv.pop(1)) if len(sys.argv) > 1 else Path(__file__).resolve().parents[1] / "ygg-memory.py"
spec = importlib.util.spec_from_file_location("native_watermark_runner", RUNNER)
ym = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ym)


class NativeWatermarkIO(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / "hub"
        self.native = Path(self.tmp.name) / "native"
        self.native.mkdir()
        for i in range(12):
            (self.native / f"note-{i:02}.md").write_text(f"Synthetic note {i}\n")
        self.sync()

    def sync(self):
        lock = ym._flock_open(self.root / ".ygg-memory.lock")
        try:
            return ym._sync_native_tree(self.root, "zcode", "_global", self.native,
                                        "memory", target_harness="all")
        finally:
            ym._flock_close(lock)

    def test_noop_tree_loads_once_and_never_rewrites_watermark(self):
        checkpoint = ym.get_watermark_path(self.root, "zcode")
        before = checkpoint.stat().st_mtime_ns
        with patch.object(ym, "load_watermark", wraps=ym.load_watermark) as loads, \
                patch.object(ym, "save_watermark", wraps=ym.save_watermark) as saves:
            self.assertEqual(self.sync(), (0, 0, 0))
        self.assertEqual(loads.call_count, 1)
        self.assertEqual(saves.call_count, 0)
        self.assertEqual(checkpoint.stat().st_mtime_ns, before)

    def test_one_edit_is_checkpointed_and_next_pass_is_quiet(self):
        doc = self.native / "note-00.md"
        doc.write_text("Changed synthetic note\n")
        with patch.object(ym, "save_watermark", wraps=ym.save_watermark) as saves:
            self.assertEqual(self.sync()[0], 1)
        self.assertEqual(saves.call_count, 1)
        disk = ym.load_watermark(self.root, "zcode")
        states = list(disk["native_documents"].values())
        self.assertIn(ym.text_sha256("Changed synthetic note"),
                      [s.get("delivered_payload_digest") for s in states])
        journal = ym.get_journal_path(self.root).read_bytes()
        self.assertEqual(self.sync(), (0, 0, 0))
        self.assertEqual(ym.get_journal_path(self.root).read_bytes(), journal)

    def test_completed_document_survives_later_document_failure(self):
        (self.native / "note-00.md").write_text("Durable edit before failure\n")
        original = ym._sync_native_document
        calls = 0

        def fail_second(*args, **kwargs):
            nonlocal calls
            calls += 1
            if calls == 2:
                raise OSError("synthetic later-document failure")
            return original(*args, **kwargs)

        with patch.object(ym, "_sync_native_document", side_effect=fail_second):
            with self.assertRaisesRegex(OSError, "later-document"):
                self.sync()
        states = ym.load_watermark(self.root, "zcode")["native_documents"]
        self.assertIn(ym.text_sha256("Durable edit before failure"),
                      [s.get("delivered_payload_digest") for s in states.values()])
        self.assertEqual(self.sync(), (0, 0, 0))

    def test_later_pass_reads_fresh_checkpoint(self):
        watermark = ym.load_watermark(self.root, "zcode")
        watermark["other_writer_marker"] = "keep across passes"
        ym.save_watermark(self.root, watermark)
        (self.native / "note-00.md").write_text("Another edit\n")
        self.sync()
        self.assertEqual(ym.load_watermark(self.root, "zcode")["other_writer_marker"],
                         "keep across passes")


if __name__ == "__main__":
    unittest.main()
