#!/usr/bin/env python3
"""Command-local dream observation equivalence and bounded-work regression."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
import warnings
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "memory_canary_batch", Path(__file__).resolve().parents[1] / "ygg-memory.py")
m = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(m)
NOW = 1000000.0


def event(seq, ns, ts=900000.0, **extra):
    return dict(seq=seq, ns=ns, ts=ts, file="notes.md", summary="write", **extra)


class BatchTests(unittest.TestCase):
    def setUp(self):
        scratch = Path.home() / ".yggterm" / "scratchpad" / "memory-canary-tests"
        scratch.mkdir(parents=True, exist_ok=True)
        self.tmp = tempfile.TemporaryDirectory(dir=scratch)
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.clock = patch.object(m.time, "time", return_value=NOW)
        self.clock.start()
        self.addCleanup(self.clock.stop)
        self.home = patch.object(m.Path, "home", return_value=self.root / "home")
        self.home.start()
        self.addCleanup(self.home.stop)

    def write(self, records, tail=""):
        (self.root / "journal.jsonl").write_text(
            "".join(json.dumps(r) + "\n" for r in records) + tail)

    def watermark(self, ns, seq=0, **extra):
        d = self.root / "dream" / ns
        d.mkdir(parents=True, exist_ok=True)
        (d / "watermark.json").write_text(json.dumps(dict(last_seq=seq, **extra)))

    def equivalent(self, names):
        view = m._DreamJournalView(self.root)
        for ns in names:
            with self.subTest(ns=ns):
                self.assertEqual(m.load_dream_watermark(self.root, ns),
                                 m.load_dream_watermark(self.root, ns, view=view))
                self.assertEqual(m._dream_pending_summary(self.root, ns),
                                 m._dream_pending_summary(self.root, ns, view=view))
                self.assertEqual(m.dream_quiescence_violation(self.root, ns),
                                 m.dream_quiescence_violation(self.root, ns, view=view))
        return view

    def test_many_namespaces_floors_duplicates_and_global(self):
        names = [f"-project-{i}" for i in range(40)] + ["_global", "-empty"]
        rows = [event(1, "_global"), event(1, "_global", NOW - 2000)]
        for i, ns in enumerate(names[:-2]):
            self.watermark(ns, i % 3)
            rows.extend([event(3, ns), event(3, ns, NOW - 2000),
                         dict(seq=4, ns=ns, ts=NOW, file="NOW.md", origin="dreamer",
                              summary="dream rollup [consumed seq 2]")])
        self.write(rows)
        view = self.equivalent(names)
        self.assertEqual(view.pending("_global", 0)["pending"], 2)
        self.assertEqual(view.pending(names[0], 2)["pending"], 2)
        self.assertEqual(view.pending("-empty", 0)["pending"], 2)

    def test_empty_no_history_corrupt_scratch_and_foreign_floor(self):
        self.equivalent(["_global", "-empty"])
        self.watermark("-one", 12)
        (self.root / "dream" / "-one" / "watermark.json").write_text("broken")
        rows = [dict(seq=0, ns="-one", ts=NOW, file="NOW.md", origin="dreamer",
                     summary="dream rollup [consumed seq 8]"), event(9, "-one")]
        self.write(rows)
        self.assertEqual(self.equivalent(["-one"]).floors["-one"], 8)
        self.watermark("-one", 20)
        self.equivalent(["-one"])

    def test_new_view_sees_append_reset_truncate_and_invalidation(self):
        self.write([event(1, "-one")])
        old = m._DreamJournalView(self.root)
        with (self.root / "journal.jsonl").open("a") as f:
            f.write(json.dumps(event(2, "-one")) + "\n")
        self.assertEqual(m._DreamJournalView(self.root).pending("-one", 0)["pending"], 2)
        self.assertEqual(old.pending("-one", 0)["pending"], 1)
        self.write([event(1, "-two")])  # same-size replacement, not mtime trust
        os.utime(self.root / "journal.jsonl", (NOW, NOW))
        self.assertEqual(self.equivalent(["-one", "-two"]).pending("-one", 0)["pending"], 0)
        m._ji_invalidate(self.root)
        self.equivalent(["-one", "-two"])
        self.write([])
        self.assertEqual(m._DreamJournalView(self.root).pending("-two", 0)["pending"], 0)

    def test_corrupt_partial_and_index_failure_fallback(self):
        self.write([event(1, "-one")], '{broken\n{"seq":')
        self.equivalent(["-one"])
        with patch.object(m, "_ji_read", side_effect=RuntimeError("unavailable index")):
            self.assertEqual(m._DreamJournalView(self.root).pending("-one", 0)["pending"], 1)
        m._journal_index_path(self.root).write_text("invalid storage")
        self.equivalent(["-one"])

    def canary(self, batch):
        warnings.simplefilter("ignore", ResourceWarning)
        out, err = io.StringIO(), io.StringIO()
        real = m._dream_pending_summary
        def summary(root, ns, **kwargs):
            return real(root, ns, **(kwargs if batch else {}))
        with patch.object(m, "_dream_pending_summary", side_effect=summary), \
                contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            try:
                m.cmd_canary(SimpleNamespace(root=str(self.root), json=True))
                rc = 0
            except SystemExit as e:
                rc = e.code
        return rc, out.getvalue(), err.getvalue()

    def test_full_canary_verdict_equivalence(self):
        rows = [event(1, "_global")]
        for i in range(25):
            ns = f"-project-{i}"
            self.watermark(ns, dreams=[{"seq": 3}, {"seq": 2}])
            rows.append(event(i + 2, ns))
        self.write(rows)
        dream = self.root / "dream"
        (dream / "auto-backoff.json").write_text(json.dumps(dict(ts=NOW, reason="fixture failure")))
        (dream / "receipts.jsonl").write_text(json.dumps(dict(ts=1)) + "\n")
        nsdir = self.root / "namespaces" / "-project-0"
        nsdir.mkdir(parents=True)
        (nsdir / "NOW.md").write_text("x" * (m.DREAM_NOW_MAX_BYTES + 513))
        mem = self.root / "home" / ".zcode/cli/memories/projects/example/memory"
        mem.mkdir(parents=True)
        (mem / "MEMORY.md").write_text("line\n" * 401)
        (mem.parent / "memory-archive").mkdir()
        self.assertEqual(self.canary(True), self.canary(False))
        rc, _, err = self.canary(True)
        self.assertEqual(rc, 1)
        for needle in ["not monotonic", "over budget", "FAILURE", "BLOWN", "re-bloated", "pointer is missing"]:
            self.assertIn(needle, err)
        (dream / "auto-backoff.json").unlink()
        self.assertEqual(self.canary(True), self.canary(False))
        self.assertIn("no execution receipt", self.canary(True)[2])

    def test_healthy_canary_and_quiescence_heartbeat(self):
        self.watermark("-one")
        self.assertEqual(self.canary(True), (0, '{"status": "ok"}\n', ""))
        self.write([event(1, "-one", NOW - 10)])
        self.equivalent(["-one", "_global"])
        with patch.object(m, "dream_autoclaw_activity_age", return_value=10):
            self.write([event(1, "-one")])
            self.equivalent(["-one"])

    def test_missing_timestamp_keeps_query_time_default(self):
        row = event(1, "-one")
        del row["ts"]
        self.write([row, event(2, "-one", NOW + 10)])
        view = m._DreamJournalView(self.root)
        with patch.object(m.time, "time", return_value=NOW + 5):
            self.assertEqual(view.pending("-one", 0), m._dream_pending_summary(self.root, "-one"))
            self.watermark("-one", 1)
            self.assertEqual(view.pending("-one", 1), m._dream_pending_summary(self.root, "-one"))

    def test_new_arrivals_do_not_mask_old_pending_work(self):
        self.watermark("-one")
        self.write([event(1, "-one", NOW - 28800),
                    event(2, "_global", NOW - 60)])
        view = self.equivalent(["-one", "_global"])
        for result in [m._dream_pending_summary(self.root, "-one"),
                       view.pending("-one", 0)]:
            self.assertEqual(result["pending"], 2)
            self.assertEqual(result["oldest_pending_s"], 28800)
            self.assertTrue(result["overdue"])
        self.assertIn("BLOWN", self.canary(True)[2])
        self.watermark("-one", 1)
        self.assertEqual(m._dream_pending_summary(self.root, "-one"),
                         {"pending": 1, "oldest_pending_s": 60, "overdue": False})

    def test_candidate_selection_and_fresh_fences(self):
        self.watermark("-old")
        self.watermark("-new")
        self.write([event(1, "-old"), event(2, "-new", NOW - 2000)])
        view = m._DreamJournalView(self.root)
        with patch.object(m.subprocess, "run", return_value=SimpleNamespace(
                returncode=0, stdout="clean tick", stderr="")) as child:
            result = m._dream_auto(self.root, "", SimpleNamespace())
            self.assertIn("[-old]", result)
            self.assertIn("--ns=-old", child.call_args.args[0])
        with (self.root / "journal.jsonl").open("a") as f:
            f.write(json.dumps(event(3, "-old", NOW)) + "\n")
        self.assertEqual(m.dream_quiescence_violation(self.root, "-old", view=view), "")
        self.assertIn("ACTIVE", m.dream_quiescence_violation(self.root, "-old"))
        self.assertIn("journal advanced", m.dream_fence_violation(self.root, "-old", 2))

    def test_operation_count_history_plus_namespaces(self):
        self.write([event(i + 1, "_global") for i in range(200)])
        for i in range(60):
            self.watermark(f"-project-{i}", 200)
        # Warm index; count payload JSON decodes and journal reader calls.
        m.read_journal_entries(self.root)
        with patch.object(m, "read_journal_entries", wraps=m.read_journal_entries) as reads, \
                patch.object(m.json, "loads", wraps=json.loads) as decodes:
            self.assertEqual(self.canary(True)[0], 0)
            self.assertEqual(reads.call_count, 1)
            self.assertLessEqual(decodes.call_count, 200 + 3 * 60)
        with patch.object(m, "read_journal_entries", wraps=m.read_journal_entries) as reads, \
                patch.object(m, "subprocess") as subprocess:
            result = m._dream_auto(self.root, "", SimpleNamespace())
            self.assertIn("no candidate", result)
            self.assertEqual(reads.call_count, 1)
            subprocess.run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
