"""Hermetic watch/status regressions; no live service or provider calls."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("watch_under_test", Path(__file__).resolve().parents[1] / "ygg-dream-orchestrator.py")
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

class WatchTests(unittest.TestCase):
    def test_unreviewed_and_submitted_work_suppresses_duplicate_watch(self):
        for state in ["review_pending", "retained_partial", "submitted", "future-state"]:
            with self.subTest(state=state):
                self.assertTrue(m.ux_watch_has_open_work([{"title": "UX-PARITY invented", "status": state}]))
        for state in ["completed", "failed", "cancelled"]:
            self.assertFalse(m.ux_watch_has_open_work([{"title": "UX-PARITY invented", "status": state}]))
        self.assertFalse(m.ux_watch_has_open_work([]))
        self.assertTrue(m.ux_watch_has_open_work(None))
    def status(self, rows, as_json):
        cfg = {"targets": {"day": 2, "moderate": 1}, "routes": []}
        with patch.object(m, "load_state", return_value={}), patch.object(m, "plane_gates", return_value={}), patch.object(m, "qjson", return_value=rows), patch.object(m, "resolve_policy", return_value={"overlays": [], "blocked_routes": [], "debt_route": None}), contextlib.redirect_stdout(io.StringIO()) as output:
            rc = m.cmd_status(cfg, as_json)
        return rc, output.getvalue()
    def test_unknown_status_cannot_look_like_healthy_empty_queue(self):
        rc, output = self.status(None, True)
        q = json.loads(output)["queue"]
        self.assertEqual(rc, 3)
        self.assertFalse(q["observed"])
        self.assertIsNone(q["backlog"])
        self.assertIsNone(q["running"])
        rc, output = self.status(None, False)
        self.assertEqual(rc, 3)
        self.assertIn("queue=UNKNOWN", output)
        self.assertNotIn("backlog=0", output)
    def test_held_debt_is_visible_separately_from_eligible_backlog(self):
        rows = [{"status": s} for s in ["queued", "review_pending", "retained_partial", "submitted"]]
        rc, output = self.status(rows, True)
        q = json.loads(output)["queue"]
        self.assertEqual(rc, 0)
        self.assertEqual(q["backlog"], 1)
        self.assertEqual(q["review_pending"], 1)
        self.assertEqual(q["retained_partial"], 1)
        self.assertEqual(q["submitted"], 1)
    def test_main_propagates_unknown_exit(self):
        with patch.object(m, "load_cfg", return_value={}), patch.object(m, "load_swarmconfig", return_value=None), patch.object(m, "cmd_status", return_value=3), patch.object(m.sys, "argv", ["watch", "status", "--json"]):
            self.assertEqual(m.main(), 3)
    def test_future_state_is_partial_instead_of_false_empty(self):
        rc, output = self.status([{"status": "future-state"}], True)
        q = json.loads(output)["queue"]
        self.assertEqual(rc, 3)
        self.assertEqual(q["total"], 1)
        self.assertEqual(q["unclassified"], 1)
        self.assertFalse(q["complete"])
    def test_unknown_schema_holds_actual_topup_before_staging(self):
        with patch.object(m, "SWCFG", None), patch.object(m, "load_state", return_value={}), patch.object(m, "qjson", return_value=[{"status": "future-state"}]), patch.object(m, "stage_brief") as stage, contextlib.redirect_stdout(io.StringIO()) as output:
            m.cmd_topup({})
        stage.assert_not_called()
        self.assertIn("queue schema UNKNOWN", output.getvalue())

if __name__ == "__main__": unittest.main()
