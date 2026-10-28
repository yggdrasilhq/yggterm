#!/usr/bin/env python3
"""Run the real router in an isolated installation with recording backends.

YGG_TEST_ROUTER may name an older router to demonstrate the help falsifier.
No live fleet, credential store or session is used.
"""
import importlib.util
import io
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

SKILL = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SKILL))
import ygg_command_tree as commands

SHIM = '''#!/usr/bin/env python3
import json,os,sys
with open(os.environ["YGG_TEST_CALLS"],"a") as f:
 f.write(json.dumps({"tool":os.path.basename(sys.argv[0]),"args":sys.argv[1:]})+"\\n")
print("backend executed")
'''


class CommandContract(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.calls = self.root / "calls.jsonl"
        self.env = dict(os.environ, YGG_BIN_DIR=str(self.bin), YGG_TEST_CALLS=str(self.calls),
                        PYTHONDONTWRITEBYTECODE="1", PATH=str(self.bin) + os.pathsep + os.environ["PATH"])
        for name in ("ygg-inspect.py", "ygg_command_tree.py", "commands.json"):
            shutil.copy2(SKILL / name, self.bin / name)
        shutil.copy2(Path(os.environ.get("YGG_TEST_ROUTER", str(SKILL / "ygg"))), self.bin / "ygg")
        for target in {n.legacy[0] for n in commands.TREE.values() if n.legacy} | {"msgboard", "ssh", "zcode-tui"}:
            if target.startswith("@"):
                continue
            p = self.bin / target
            p.write_text(SHIM)
            p.chmod(0o755)
        self.env["YGG_MEMORY_BIN"] = str(self.bin / "ygg-memory")

    def run_cli(self, *args):
        return subprocess.run(["bash", str(self.bin / "ygg"), *args], env=self.env,
                              cwd=self.root, capture_output=True, text=True, timeout=5)

    def test_help_at_every_prefix_cannot_reach_a_backend(self):
        for path in commands.TREE:
            with self.subTest(path=path):
                r = self.run_cli(*path, "--help")
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertIn("ygg", r.stdout)
                self.assertFalse(self.calls.exists(), r.stdout)
        for path in ((), ("dream",), ("service",), ("schedule",), ("swarm",), ("board",)):
            with self.subTest(incomplete=path):
                r = self.run_cli(*path)
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertIn("Next words", r.stdout)
                self.assertFalse(self.calls.exists())

    def test_invalid_new_arguments_never_reach_a_backend(self):
        for args in (("dream", "topup", "--typo"), ("dream", "ls", "--state", "made-up"),
                     ("dream", "inspect"), ("dream", "open"),
                     ("dream", "open", "dream-42", "--pick"),
                     ("ygg-dream-orchestrator.py", "topup", "--typo"),
                     ("ygg-fleet.py", "agents", "ps", "--opaque-owner-argument"),
                     ("ygg-fleet.py", "fleet", "pressure", "--typo"),
                     ("swarm", "pressure", "--lim", "2"),
                     ("service", "ps", "--limit", "0"), ("service", "ps", "--limit", "bad")):
            with self.subTest(args=args):
                r = self.run_cli(*args)
                self.assertEqual(r.returncode, 2, (r.stdout, r.stderr))
                self.assertFalse(self.calls.exists())

    def test_examples_and_completion_are_local_and_use_same_tree(self):
        for path, example in commands.examples():
            parsed, _, _ = commands.parse(shlex.split(example.command)[1:])
            self.assertEqual(parsed, path)
        r = self.run_cli("help", "examples", "--search", "unused quota", "--json")
        self.assertEqual(r.returncode, 0, r.stderr)
        found = json.loads(r.stdout)["examples"]
        self.assertTrue(any("swarm pressure" in e["command"] for e in found))
        r = self.run_cli("complete", "--", "dream", "ls", "--state", "f")
        self.assertEqual(r.stdout.strip(), "failed")
        self.assertFalse(self.calls.exists())
        snippet = self.run_cli("completion", "bash")
        self.assertEqual(subprocess.run(["bash", "-n"], input=snippet.stdout, text=True).returncode, 0)

    def test_owned_handlers_keep_arguments_and_board_routes_to_topic_tool(self):
        cases = [(("dream", "status", "--json"), "ygg-dream-orchestrator.py", ["status", "--json"]),
                 (("dream", "topup", "--json"), "ygg-dream-orchestrator.py", ["topup", "--json"]),
                 (("ygg-dream-orchestrator.py", "topup", "--json"), "ygg-dream-orchestrator.py", ["topup", "--json"]),
                 (("ygg-fleet.py", "agents", "ps", "--json"), "ygg-fleet.py", ["agents", "ps", "--json"]),
                 (("ygg-fleet.py", "agents", "--json"), "ygg-fleet.py", ["agents", "--json"]),
                 (("ci", "status", "--json"), "ygg-ci.py", ["status", "--json"]),
                 (("memory", "status", "--json"), "ygg-memory", ["status", "--json"]),
                 (("row", "list", "--json"), "ygg_row.py", ["list", "--json"]),
                 (("ygg_row.py", "list"), "ygg_row.py", ["list"]),
                 (("claim", "--purpose", "synthetic task"), "ygg-claim.sh", ["--purpose", "synthetic task"]),
                 (("board", "read", "infra/example"), "msgboard", ["read", "infra/example"]),
                 (("board", "rows", "--json"), "ygg-board.py", ["--json"])]
        cases.append((("board", "--json"), "ygg-board.py", ["--json"]))
        for args, tool, expected in cases:
            with self.subTest(args=args):
                self.calls.unlink(missing_ok=True)
                r = self.run_cli(*args)
                self.assertEqual(r.returncode, 0, r.stderr)
                records = [json.loads(l) for l in self.calls.read_text().splitlines()]
                self.assertEqual(records, [{"tool": tool, "args": expected}])

    def test_missing_kernel_is_unknown_not_a_healthy_empty_ledger(self):
        r = self.run_cli("dream", "ls", "--json")
        self.assertEqual(r.returncode, 3, r.stderr)
        result = json.loads(r.stdout)
        self.assertEqual(result["coverage"][0]["quality"], "unavailable")
        self.assertEqual(result["findings"][0]["code"], "ADAPTER_UNAVAILABLE")
        self.assertFalse(self.calls.exists())

    def test_explicit_broken_kernel_cannot_use_a_different_local_kernel(self):
        package = self.bin / "introspection"
        package.mkdir()
        (package / "adapters.py").write_text(
            "import os\nfrom pathlib import Path\n"
            "def serve(path, args):\n"
            " Path(os.environ['YGG_TEST_CALLS']).write_text('wrong kernel used')\n"
            " print('{}')\n return 0\n"
        )
        control = self.run_cli("dream", "ls", "--json")
        self.assertEqual(control.returncode, 0, control.stderr)
        self.assertTrue(self.calls.exists())
        self.calls.unlink()
        self.env["YGG_KERNEL_DIR"] = str(self.root / "missing-selected-generation")
        result = self.run_cli("dream", "ls", "--json")
        self.assertEqual(result.returncode, 3, result.stderr)
        self.assertEqual(json.loads(result.stdout)["coverage"][0]["quality"], "unavailable")
        self.assertFalse(self.calls.exists())

    def test_packaged_kernel_alias_is_standalone_regular_file(self):
        spec = importlib.util.spec_from_file_location("disperse", SKILL / "ygg-disperse.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        shipment, files = module.load_shipment(SKILL)
        with tarfile.open(fileobj=io.BytesIO(module.stage_tar(shipment, files, SKILL))) as archive:
            member = archive.getmember("ygg_command_tree.py")
            self.assertTrue(member.isfile(), "source alias must be dereferenced in remote shipment")
            target = self.root / "shipment"
            archive.extractall(target, filter="data")
        self.assertFalse((target / "ygg_command_tree.py").is_symlink())
        self.assertEqual((target / "ygg_command_tree.py").read_bytes(), (SKILL / "ygg_command_tree.py").read_bytes())
        env = dict(self.env, YGG_BIN_DIR=str(target))
        r = subprocess.run(["bash", str(target / "ygg"), "dream", "--help"],
                           env=env, cwd=self.root, capture_output=True, text=True, timeout=5)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("Next words", r.stdout)

    def test_board_scan_consumes_source_json_lines_and_answer_edges(self):
        (self.bin / "msgboard").write_text('''#!/usr/bin/env python3
import os,sys
print(os.environ["YGG_TEST_BOARD_DATA"])
print(os.environ.get("YGG_TEST_BOARD_WARNING", ""),file=sys.stderr)
''')
        posts = [{"kind": "note", "ack_token": "ACK-a", "body": "First", "refs": []},
                 {"kind": "note", "ack_token": "ACK-b", "body": "Second", "refs": []},
                 {"kind": "answer", "ack_token": "ACK-c", "body": "Built first", "refs": ["ACK-a"]}]
        self.env["YGG_TEST_BOARD_DATA"] = "\n".join(json.dumps(p) for p in posts)
        def scan():
            return subprocess.run([sys.executable, str(SKILL / "ygg-fleet.py"), "board", "--json"],
                                  env=self.env, capture_output=True, text=True, timeout=5)
        result = scan()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([p["ack_token"] for p in json.loads(result.stdout)["open_dreams"]], ["ACK-b"])
        self.env["YGG_TEST_BOARD_WARNING"] = "source skipped a malformed record"
        result = scan()
        self.assertEqual(result.returncode, 3)
        self.assertEqual(json.loads(result.stdout)["quality"], "unknown")
        self.env["YGG_TEST_BOARD_WARNING"] = ""
        self.env["YGG_TEST_BOARD_DATA"] = json.dumps(posts[1]) + "\nnot JSON"
        result = scan()
        self.assertEqual(result.returncode, 3)
        self.assertEqual([p["ack_token"] for p in json.loads(result.stdout)["open_dreams"]], ["ACK-b"])
        self.env["YGG_TEST_BOARD_DATA"] = "not JSON"
        result = scan()
        self.assertEqual(result.returncode, 3)
        self.assertEqual(json.loads(result.stdout)["quality"], "unknown")


if __name__ == "__main__":
    unittest.main()
