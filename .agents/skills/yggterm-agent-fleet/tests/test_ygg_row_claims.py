"""Hermetic process-level seat allocation controls; no live registry."""
import concurrent.futures
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "ygg_row.py"

class ClaimTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.claims = self.root / "rows/claims"
        self.claims.mkdir(parents=True)
        (self.root / "rows/campaigns.json").write_text(
            json.dumps({"7": {"name": "invented"}}))
    def tearDown(self):
        self.temp.cleanup()
    def run_claim(self, sid, *args):
        env = dict(os.environ, YGGTERM_HOME=str(self.root), YGGTERM_SESSION_ID=sid)
        return subprocess.run([sys.executable, str(SCRIPT), "claim", "--campaign", "7", *args], env=env, capture_output=True, text=True, timeout=20)
    def records(self):
        return [json.loads(p.read_text()) for p in self.claims.glob("*.json")]
    def test_sequential_and_idempotent(self):
        self.assertEqual(self.run_claim("first").returncode, 0)
        original = (self.claims / "7.1.json").read_bytes()
        self.assertEqual(self.run_claim("second").returncode, 0)
        self.assertEqual(self.run_claim("first").returncode, 0)
        self.assertEqual((self.claims / "7.1.json").read_bytes(), original)
        self.assertEqual({r["id"] for r in self.records()}, {"7.1", "7.2"})
    def test_parallel_publication(self):
        with concurrent.futures.ThreadPoolExecutor(max_workers=12) as ex:
            results = list(ex.map(self.run_claim, [f"worker-{i}" for i in range(12)]))
        self.assertTrue(all(p.returncode == 0 for p in results), [p.stderr for p in results])
        records = self.records()
        self.assertEqual(len(records), 12)
        self.assertEqual(len({r["session"] for r in records}), 12)
    def test_existing_unknown_seat_retained(self):
        p = self.claims / "7.1.json"
        p.write_bytes(b"UNKNOWN FUTURE SEAT")
        self.assertEqual(self.run_claim("new").returncode, 0)
        self.assertEqual(p.read_bytes(), b"UNKNOWN FUTURE SEAT")
        self.assertTrue((self.claims / "7.2.json").exists())
    def test_orchestrator_collision_and_cross_campaign(self):
        self.assertEqual(self.run_claim("master", "--role", "orchestrator").returncode, 0)
        p = self.claims / "7.0.json"
        before = p.read_bytes()
        self.assertNotEqual(self.run_claim("other", "--role", "orchestrator").returncode, 0)
        self.assertNotEqual(self.run_claim("wrong", "--id", "8.1").returncode, 0)
        self.assertEqual(p.read_bytes(), before)
        self.assertFalse((self.claims / "8.1.json").exists())
    def test_parallel_same_explicit_seat(self):
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as ex:
            results = list(ex.map(lambda s: self.run_claim(s, "--id", "7.3"), ["one", "two"]))
        self.assertEqual(sum(r.returncode == 0 for r in results), 1)
        self.assertEqual(len(self.records()), 1)
    def test_dangling_explicit_seat_retained(self):
        p = self.claims / "7.3.json"
        p.symlink_to("absent")
        self.assertNotEqual(self.run_claim("new", "--id", "7.3").returncode, 0)
        self.assertTrue(p.is_symlink())
    def test_nonobject_occupied_json_is_opaque(self):
        for value in [[], ["future"], "future", 9, True, None]:
            with self.subTest(value=value):
                p = self.claims / "7.1.json"
                raw = json.dumps(value).encode()
                p.write_bytes(raw)
                self.assertEqual(self.run_claim("new").returncode, 0)
                self.assertEqual(p.read_bytes(), raw)
                self.assertTrue((self.claims / "7.2.json").exists())
                (self.claims / "7.2.json").unlink()

if __name__ == "__main__":
    unittest.main()
