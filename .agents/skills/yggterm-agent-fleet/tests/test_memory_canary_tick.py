#!/usr/bin/env python3
"""Execute the actual tick with isolated paths and a deterministic fake CLI."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


TICK = Path(__file__).resolve().parents[1] / "ygg-memory-tick"
FAKE = '''
import json, os, sys
from pathlib import Path
root = Path(os.environ["YGG_CANARY_TEST_HOME"])
path = root / "calls.jsonl"
rows = [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
operation = sys.argv[1]
count = sum(row["operation"] == "canary" for row in rows) + 1
with path.open("a") as output:
    output.write(json.dumps({"operation": operation}) + "\\n")
mode = os.environ["YGG_CANARY_TEST_MODE"]
if operation == "sync-harness" and mode == "sync-fails":
    raise SystemExit(4)
if operation == "canary":
    if mode == "first-fails" and count == 1:
        print("canary: invented first failure", file=sys.stderr)
        raise SystemExit(7)
    if mode == "empty-failure":
        raise SystemExit(9)
    print('{"status":"ok"}')
'''


class TickTests(unittest.TestCase):
    def run_tick(self, mode):
        scratch = Path.home() / ".yggterm/scratchpad/memory-tick-tests"
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as folder:
            root = Path(folder)
            executable = root / ".local/bin/ygg-memory"
            executable.parent.mkdir(parents=True)
            executable.write_text("#!" + sys.executable + "\n" + FAKE)
            executable.chmod(0o700)
            config = root / ".config/ygg-fleet/memory-sync.conf"
            config.parent.mkdir(parents=True)
            config.write_text("harnesses=example-a,example-b\n")
            (root / ".yggterm/memory").mkdir(parents=True)
            # Redirect only the environment lookup name in the real script;
            # never replace HOME or execute a duplicate of its control logic.
            script = TICK.read_text().replace("${HOME}", "${YGG_CANARY_TEST_HOME}")
            script = script.replace("$HOME", "$YGG_CANARY_TEST_HOME")
            fixture = root / "tick.sh"
            fixture.write_text(script)
            env = dict(os.environ, YGG_CANARY_TEST_HOME=str(root),
                       YGG_CANARY_TEST_MODE=mode)
            result = subprocess.run(["bash", str(fixture)], env=env,
                                    capture_output=True, text=True, timeout=10)
            calls = [json.loads(line) for line in (root / "calls.jsonl").read_text().splitlines()]
            log = root / ".yggterm/memory/canary.log"
            return result, calls, log.read_text() if log.exists() else None

    def test_healthy_canary_runs_once_without_failure_log(self):
        result, calls, log = self.run_tick("healthy")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(sum(row["operation"] == "canary" for row in calls), 1)
        self.assertEqual(sum(row["operation"] == "sync-harness" for row in calls), 4)
        self.assertIsNone(log)

    def test_first_failure_is_retained_without_a_second_probe(self):
        result, calls, log = self.run_tick("first-fails")
        self.assertEqual(result.returncode, 0)  # Original tick policy: log health faults.
        self.assertEqual(sum(row["operation"] == "canary" for row in calls), 1)
        self.assertIn("canary exit 7", log)
        self.assertIn("invented first failure", log)
        self.assertNotIn('"status":"ok"', log)

    def test_empty_failed_probe_still_has_exit_evidence(self):
        _, calls, log = self.run_tick("empty-failure")
        self.assertEqual(sum(row["operation"] == "canary" for row in calls), 1)
        self.assertIn("canary exit 9", log)

    def test_native_sync_failure_remains_nonzero(self):
        result, calls, log = self.run_tick("sync-fails")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(sum(row["operation"] == "canary" for row in calls), 1)
        self.assertIsNone(log)


if __name__ == "__main__":
    unittest.main()
