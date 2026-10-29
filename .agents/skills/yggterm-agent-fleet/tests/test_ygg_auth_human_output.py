#!/usr/bin/env python3
"""Human metadata and explicit JSON exercise the same sanitized auth results."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
spec = importlib.util.spec_from_file_location("auth_output", ROOT / "ygg-auth.py")
auth = importlib.util.module_from_spec(spec)
spec.loader.exec_module(auth)

STATUS = {"harness": "example", "host": "example-host", "live": None,
          "profiles": [], "native_health": "unknown"}

class HumanOutput(unittest.TestCase):
    def run_main(self, handler, result, argv):
        stream = io.StringIO()
        with patch.object(auth, handler, return_value=result), \
                patch.object(auth.os, "umask"), contextlib.redirect_stdout(stream):
            self.assertEqual(auth.main(argv), 0)
        return stream.getvalue()

    def test_all_status_is_grouped_text_and_explicit_json_is_unchanged(self):
        result = {"schema_version": 1, "harnesses": {"example": STATUS},
                  "coverage": "registered adapters only"}
        human = self.run_main("cmd_status", result, ["status", "--harness", "all"])
        self.assertIn("Auth status", human)
        self.assertIn("example", human)
        self.assertIn("native auth health: unknown", human)
        self.assertIn("Coverage: registered adapters only", human)
        self.assertNotIn('{"', human)
        machine = self.run_main("cmd_status", result, ["status", "--harness", "all", "--json"])
        self.assertEqual(json.loads(machine), result)

    def test_metadata_has_labels_and_preserves_unknown_false_and_zero(self):
        result = {"schema_version": 1, "harness": "example", "policy": {
            "enabled": False, "attempts": 0, "quota": None, "routes": []}}
        human = self.run_main("cmd_policy", result, ["policy", "status"])
        self.assertIn("enabled: no", human)
        self.assertIn("attempts: 0", human)
        self.assertIn("quota: unknown", human)
        self.assertIn("routes: none", human)
        self.assertNotIn("{", human)
        machine = self.run_main("cmd_policy", result, ["policy", "status", "--json"])
        self.assertEqual(json.loads(machine), result)

if __name__ == "__main__":
    unittest.main()
