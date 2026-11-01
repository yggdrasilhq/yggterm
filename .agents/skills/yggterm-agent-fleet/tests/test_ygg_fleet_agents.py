#!/usr/bin/env python3
"""Falsify local-PID guesses against owner-host process observations."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "fleet_agents_under_test", Path(__file__).resolve().parents[1] / "ygg-fleet.py")
fleet = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fleet)


class AgentObservationTests(unittest.TestCase):
    def run_agents(self, rows, *, as_json=True, rc=0, stderr=""):
        output = io.StringIO()
        result = (rc, json.dumps(rows), stderr)
        with patch.object(fleet, "run", return_value=result), \
                patch.object(fleet.Path, "exists", side_effect=AssertionError("local procfs guess")), \
                contextlib.redirect_stdout(output):
            code = fleet.cmd_agents(as_json)
        return code, json.loads(output.getvalue()) if as_json else output.getvalue()

    def row(self, state="ALIVE", **extra):
        return dict(id="7.2", host="node-b", pid=999999,
                    harness="example", purpose="Example work",
                    process_liveness=dict(state=state, reason="owner_host_sample"), **extra)

    def test_remote_alive_absent_local_pid_remains_alive(self):
        code, result = self.run_agents([self.row()])
        self.assertEqual(code, 0)
        self.assertEqual(result["sessions"][0]["process_liveness"]["state"], "ALIVE")
        self.assertIs(result["sessions"][0]["dead"], False)

    def test_owner_dead_is_explicit_and_legacy_flag_is_rederived(self):
        code, result = self.run_agents([self.row("DEAD", dead=False)])
        self.assertEqual(code, 0)
        self.assertIs(result["sessions"][0]["dead"], True)
        _, alive = self.run_agents([self.row(dead=True)])
        self.assertIs(alive["sessions"][0]["dead"], False)

    def test_unknown_unbound_and_missing_never_become_death(self):
        for state, expected_code in [("UNKNOWN", 3), ("UNBOUND", 0)]:
            with self.subTest(state=state):
                code, result = self.run_agents([self.row(state, dead=True)])
                self.assertEqual(code, expected_code)
                self.assertNotIn("dead", result["sessions"][0])
        row = self.row()
        row.pop("process_liveness")
        code, result = self.run_agents([row])
        self.assertEqual(code, 3)
        self.assertEqual(result["sessions"][0]["process_liveness"]["state"], "UNKNOWN")

    def test_malformed_liveness_is_unknown(self):
        for observation in [None, [], "ALIVE", {}, {"state": "ALIVE"},
                            {"state": "ALIVE", "reason": ""},
                            {"state": "RUNNING", "reason": "invented"}]:
            row = self.row()
            row["process_liveness"] = observation
            code, result = self.run_agents([row])
            self.assertEqual(code, 3)
            self.assertEqual(result["sessions"][0]["process_liveness"]["state"], "UNKNOWN")

    def test_bad_inventory_and_partial_coverage_are_nonzero(self):
        for rows in [{"sessions": []}, None, 4, "invented"]:
            code, result = self.run_agents(rows)
            self.assertEqual(code, 3)
            self.assertEqual(result["sessions"], [])
            self.assertTrue(result["errors"])
        code, result = self.run_agents([self.row(), None])
        self.assertEqual(code, 3)
        self.assertEqual(len(result["sessions"]), 1)
        code, _ = self.run_agents([self.row()], stderr="peer sample unavailable")
        self.assertEqual(code, 3)
        code, _ = self.run_agents([], rc=124, stderr="invented timeout")
        self.assertEqual(code, 3)

    def test_empty_registered_inventory_is_not_a_process_death_claim(self):
        code, result = self.run_agents([])
        self.assertEqual(code, 0)
        self.assertEqual(result, {"sessions": [], "errors": []})

    def test_human_process_column_and_control_safe_metadata(self):
        row = self.row("UNKNOWN")
        row.update(host="node-b\nFAKE", purpose="Example\x1b[31m\nINJECTED\tTITLE\u202e")
        code, text = self.run_agents([row], as_json=False)
        self.assertEqual(code, 3)
        self.assertIn("PROCESS", text)
        self.assertIn("UNKNOWN", text)
        self.assertNotIn("\x1b", text)
        self.assertNotIn("\u202e", text)
        self.assertEqual(len(text.splitlines()), 3)
        # Explicit JSON preserves the original metadata for programmatic inspection.
        _, result = self.run_agents([row])
        self.assertEqual(result["sessions"][0]["purpose"], row["purpose"])

    def test_invalid_json_and_actual_main_exit(self):
        output = io.StringIO()
        with patch.object(fleet, "run", return_value=(0, "invalid-json", "")), \
                contextlib.redirect_stdout(output):
            self.assertEqual(fleet.cmd_agents(True), 3)
        with patch.object(fleet.sys, "argv", ["ygg-fleet.py", "agents", "--json"]), \
                patch.object(fleet, "cmd_agents", return_value=3), self.assertRaises(SystemExit) as raised:
            fleet.main()
        self.assertEqual(raised.exception.code, 3)


if __name__ == "__main__":
    unittest.main()
