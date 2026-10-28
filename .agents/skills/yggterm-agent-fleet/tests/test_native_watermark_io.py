#!/usr/bin/env python3
"""Native sync must not reread/rewrite a whole harness checkpoint per door.

Run with an optional runner path to falsify an older implementation.
All data is synthetic; TMPDIR can point at disk-backed agent scratch.
"""
import importlib.util
import json
import os
import subprocess
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

    def test_noop_tree_looks_up_versions_in_one_journal_snapshot(self):
        with patch.object(ym, "version_for_digest", wraps=ym.version_for_digest) as individual, \
                patch.object(ym, "read_journal_entries", wraps=ym.read_journal_entries) as reads:
            self.assertEqual(self.sync(), (0, 0, 0))
        self.assertEqual(individual.call_count, 0)
        self.assertEqual(reads.call_count, 1)

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


class NativeVersionSnapshot(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.journal = self.root / "journal.jsonl"

    def record(self, version, seq=1, namespace="_global"):
        return {"seq": seq, "ns": namespace, "file": "note.md", "digest": "digest",
                "version_id": version}

    def write(self, records):
        self.journal.write_text("".join(json.dumps(r) + "\n" for r in records))

    def test_append_and_same_size_rewrite_invalidate_cached_versions(self):
        self.write([self.record("v1"), self.record("excluded", seq=0),
                    self.record("other-ns", namespace="-other")])
        lookup = ym._native_version_lookup(self.root, "_global")
        self.assertEqual(lookup("note.md", "digest"), "v1")
        with self.journal.open("a") as out:
            out.write(json.dumps(self.record("v2")) + "\n")
        self.assertEqual(lookup("note.md", "digest"), "v2")

        st = self.journal.stat()
        content = self.journal.read_text().replace('"v2"', '"v3"')
        self.journal.write_text(content)
        os.utime(self.journal, ns=(st.st_atime_ns, st.st_mtime_ns))
        self.assertEqual(lookup("note.md", "digest"), "v3")
        self.assertEqual(lookup("missing.md", "digest"), None)

    def test_scan_overlapping_a_write_falls_back_and_is_not_cached(self):
        self.write([self.record("v1")])
        lookup = ym._native_version_lookup(self.root, "_global")
        read = ym.read_journal_entries

        def race(*args, **kwargs):
            old = read(*args, **kwargs)
            self.write([self.record("v2")])
            return old

        with patch.object(ym, "read_journal_entries", side_effect=race), \
                patch.object(ym, "version_for_digest", wraps=ym.version_for_digest) as fallback:
            self.assertEqual(lookup("note.md", "digest"), "v2")
            self.assertEqual(fallback.call_count, 1)
        with patch.object(ym, "read_journal_entries", wraps=read) as refreshed:
            self.assertEqual(lookup("note.md", "digest"), "v2")
            self.assertEqual(refreshed.call_count, 1)

    def test_absent_journal_and_atomic_replacement_are_observed(self):
        lookup = ym._native_version_lookup(self.root, "_global")
        self.assertIsNone(lookup("note.md", "digest"))
        self.write([self.record("v1")])
        self.assertEqual(lookup("note.md", "digest"), "v1")
        incoming = self.root / "replacement.jsonl"
        incoming.write_text(json.dumps(self.record("v2")) + "\n")
        incoming.replace(self.journal)
        self.assertEqual(lookup("note.md", "digest"), "v2")


class NativeAdapterSnapshot(unittest.TestCase):
    def test_same_namespace_directories_share_only_one_adapter_pass_snapshot(self):
        with tempfile.TemporaryDirectory() as td:
            home = Path(td)
            root = home / "hub"
            adapter = ym.ZcodeMemoryAdapter(home)
            for i in range(2):
                native = adapter.projects_root / f"synthetic-{i}" / "memory"
                native.mkdir(parents=True)
                (native / f"note-{i}.md").write_text(f"Synthetic note {i}\n")
            with patch.object(ym, "sync_instruction_document", return_value=(0, 0, 0)):
                adapter.sync_all(root, "zcode")
                adapter.sync_all(root, "zcode")
                with patch.object(ym, "_native_version_lookup", wraps=ym._native_version_lookup) as snapshots, \
                        patch.object(ym, "read_journal_entries", wraps=ym.read_journal_entries) as reads:
                    adapter.sync_all(root, "zcode")
                    self.assertEqual(snapshots.call_count, 1)
                    self.assertEqual(reads.call_count, 1)
                    adapter.sync_all(root, "zcode")
                    self.assertEqual(snapshots.call_count, 2)
                    self.assertEqual(reads.call_count, 2)

    def test_foreground_namespace_gate_waits_for_a_bulk_writer(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            code = "import fcntl,sys,time; f=open(sys.argv[1],'a+'); fcntl.flock(f,fcntl.LOCK_EX); print('locked',flush=True); time.sleep(11)"
            child = subprocess.Popen([sys.executable, "-c", code, str(root / ".ygg-memory.lock")],
                                     stdout=subprocess.PIPE, text=True)
            try:
                self.assertEqual(child.stdout.readline().strip(), "locked")
                from types import SimpleNamespace
                adapter = SimpleNamespace(sync_namespace=lambda *args: (0, 0, 0))
                with patch.object(ym, "get_harness_adapter", return_value=adapter):
                    report = ym._sync_harness_namespace(root, "zcode", "-synthetic")
                self.assertEqual(report["status"], "ok")
                self.assertEqual(child.wait(timeout=2), 0)
            finally:
                if child.poll() is None:
                    child.terminate()
                    child.wait()
                child.stdout.close()


if __name__ == "__main__":
    unittest.main()
