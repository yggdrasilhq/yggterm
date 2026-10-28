#!/usr/bin/env python3
"""Native Codex input/window evidence, exact identity and fail-closed report."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from contextlib import redirect_stdout
import io
import os
import subprocess
import sys
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1] / "context-relay-gauge.py"
spec = importlib.util.spec_from_file_location("gauge", SOURCE)
gauge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gauge)
SESSION = "11111111-2222-4333-8444-555555555555"
OTHER = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"


class NativeCodexGauge(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.folder = self.root / "sessions/2026/01/01"
        self.folder.mkdir(parents=True)

    def write(self, session=SESSION, *, window=1000, used=600, cached=500):
        p = self.folder / f"rollout-{session}.jsonl"
        rows = [{"type": "session_meta", "payload": {"id": session}},
                {"type": "event_msg", "payload": {"type": "token_count", "info": {
                    "model_context_window": window,
                    "total_token_usage": {"input_tokens": 999999},
                    "last_token_usage": {"input_tokens": used, "cached_input_tokens": cached}}}}]
        p.write_text("".join(json.dumps(r) + "\n" for r in rows))
        return p

    def test_cache_is_included_once_and_cumulative_usage_is_not_context(self):
        p = self.write()
        r = gauge.codex_usage(SESSION, self.root)
        self.assertEqual(r["used"], 600)
        self.assertEqual(r["window"], 1000)
        self.assertTrue(r["cache_included_in_input"])
        with p.open("rb") as f:
            f.seek(r["usage_record_offset"])
            import hashlib
            self.assertEqual(hashlib.sha256(f.readline()).hexdigest(), r["usage_record_sha256"])

    def test_parallel_session_cannot_supply_missing_session_usage(self):
        self.write(OTHER)
        with self.assertRaisesRegex(ValueError, "missing_or_ambiguous"):
            gauge.codex_usage(SESSION, self.root)

    def test_per_request_input_wins_over_a_later_summary(self):
        p = self.write()
        with p.open("a") as f:
            f.write(json.dumps({"type":"token_usage_record","payload":{"usage":{
                "input_tokens":400,"cached_input_tokens":300}}})+"\n")
            f.write(json.dumps({"type":"event_msg","payload":{"type":"token_count","info":{
                "model_context_window":1000,"last_token_usage":{"input_tokens":900}}}})+"\n")
        r = gauge.codex_usage(SESSION, self.root)
        self.assertEqual(r["used"], 400)
        self.assertEqual(r["usage_record_kind"], "token_usage_record")

    def test_identity_mismatch_duplicate_source_missing_window_and_corruption_hold(self):
        p = self.write()
        raw = p.read_text()
        p.write_text(raw.replace(SESSION, OTHER))
        with self.assertRaisesRegex(ValueError, "identity_mismatch"):
            gauge.codex_usage(SESSION, self.root)
        p.write_text(raw)
        archive = self.root / "archived_sessions"
        archive.mkdir()
        copy = archive / p.name
        copy.write_text(raw)
        with self.assertRaisesRegex(ValueError, "ambiguous"):
            gauge.codex_usage(SESSION, self.root)
        copy.unlink()
        self.write(window=None)
        with self.assertRaisesRegex(ValueError, "window_unavailable"):
            gauge.codex_usage(SESSION, self.root)
        p = self.write()
        with p.open("a") as f:
            f.write('{"incomplete":')
        with self.assertRaisesRegex(ValueError, "corrupt_or_partial"):
            gauge.codex_usage(SESSION, self.root)
        p = self.write()
        with p.open("a") as f:
            f.write(json.dumps({"type":"event_msg","payload":{"type":"token_count","info":{
                "last_token_usage":{"input_tokens":600},"model_context_window":None}}})+"\n")
        with self.assertRaisesRegex(ValueError, "window_unavailable"):
            gauge.codex_usage(SESSION, self.root)

    def test_report_publishes_readback_and_unknown_is_not_zero_success(self):
        self.write()
        with patch.object(gauge, "PUBLISH", self.root / "published"), \
                patch.dict(gauge.os.environ, {"CODEX_HOME": str(self.root)}):
            self.assertEqual(gauge.run_codex(SESSION, report=True), 0)
            r = json.loads((self.root / "published" / f"{SESSION}.json").read_text())
            self.assertEqual(r["pct"], 60)
            self.assertEqual(r["verdict"], "NOTICE")
            self.assertEqual(gauge.run_codex(OTHER, report=True), 3)
            r = json.loads((self.root / "published" / f"{OTHER}.json").read_text())
            self.assertIsNone(r["pct"])
            self.assertEqual(r["verdict"], "UNKNOWN")
            self.assertEqual(gauge.run_codex(SESSION, report=True, window_override=2000), 3)

    def test_malformed_event_payload_cannot_leave_a_verified_old_reading(self):
        for payload in ([], None, False, "malformed"):
            with self.subTest(payload=payload):
                p = self.write()
                with p.open("a") as f:
                    f.write(json.dumps({"type":"event_msg","payload":payload})+"\n")
                with self.assertRaisesRegex(ValueError, "payload"):
                    gauge.codex_usage(SESSION, self.root)

    def test_unrepresentable_native_number_is_unknown_in_json_mode(self):
        self.write(used=10**400)
        with patch.object(gauge,"PUBLISH",self.root/"published"), \
                patch.dict(gauge.os.environ,{"CODEX_HOME":str(self.root)}), redirect_stdout(io.StringIO()) as out:
            self.assertEqual(gauge.run_codex(SESSION, as_json=True), 3)
            self.assertEqual(json.loads(out.getvalue())["quality"], "unknown")

    def test_initialization_error_is_strict_for_cli_and_nonfatal_for_hook(self):
        for flags, expected in (([], 0), (["--report"], 3), (["--json"], 3), (["--report", "--json"], 3)):
            with self.subTest(flags=flags):
                result = subprocess.run([sys.executable,str(SOURCE),"--cli","codex","--session",SESSION,*flags],
                    env=dict(os.environ,CTX_NOTICE="invalid-threshold"),capture_output=True,text=True,timeout=5)
                self.assertEqual(result.returncode,expected,result.stderr)
                self.assertNotIn("Traceback",result.stderr)


if __name__ == "__main__":
    unittest.main()
