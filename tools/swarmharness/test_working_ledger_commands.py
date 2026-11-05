"""Exercise the real sentence dispatcher and the local notebook owner."""
import importlib.util
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
PAYLOAD = REPO / ".agents/skills/yggterm-agent-fleet"
sys.path.insert(0, str(HERE))
from introspection import commands
import working_ledger_cli


class SentenceIntegration(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ledger-sentences-")
        self.home = Path(self.temp.name)
        self.root = self.home / ".ygg/ledgers"
        self.env = {**os.environ, "HOME": str(self.home),
                    "YGG_LEDGER_ROOT": str(self.root),
                    "YGG_BIN_DIR": str(PAYLOAD),
                    "PYTHONDONTWRITEBYTECODE": "1"}

    def tearDown(self):
        self.temp.cleanup()

    def run_sentence(self, *words, env=None):
        return subprocess.run(
            [sys.executable, str(PAYLOAD / "ygg-inspect.py"), *words],
            env=env or self.env, text=True, capture_output=True, timeout=20,
        )

    def test_all_partial_help_routes_are_local_and_pure(self):
        poison = {**self.env, "YGG_BIN_DIR": str(self.home / "missing-owner"),
                  "YGG_KERNEL_DIR": str(self.home / "missing-kernel")}
        for path in commands.TREE:
            if path[:1] == ("ledger",):
                result = self.run_sentence(*path, "--help", env=poison)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("ygg ledger", result.stdout)
        self.assertFalse(self.root.exists())

    def test_catalog_examples_parse_in_the_actual_owner(self):
        parser = working_ledger_cli.build_parser()
        for path, example in commands.examples(("ledger",)):
            words = shlex.split(example.command)[1:]
            resolved, _, rest = commands.parse(words)
            node = commands.TREE[resolved]
            self.assertEqual(node.component, "working_ledger")
            parser.parse_args([*node.legacy[1:], *rest])

    def test_human_default_json_flag_alias_and_literal_note(self):
        result = self.run_sentence("ledger", "create", "--title", "Example notebook")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("created entry", result.stdout)
        self.assertFalse(result.stdout.lstrip().startswith("{"))
        listed = self.run_sentence("ledger", "ls", "--json")
        self.assertEqual(listed.returncode, 0, listed.stderr)
        slug = json.loads(listed.stdout)["data"]["entries"][0]["slug"]
        marker = self.home / "must-not-exist"
        note = "$(touch " + str(marker) + "); literal observation"
        result = self.run_sentence("ledger", "note", "--entry", slug, "--text", note)
        self.assertEqual(result.returncode, 0, result.stderr)
        shown = self.run_sentence("ledger", "show", "--entry", slug, "--json")
        self.assertEqual(json.loads(shown.stdout)["data"]["notes"]["items"][0]["text"], note)
        self.assertFalse(marker.exists())

    def test_missing_required_words_cannot_create_a_store(self):
        result = self.run_sentence("ledger", "create")
        self.assertEqual(result.returncode, 2)
        self.assertIn("--title", result.stderr)
        self.assertFalse(self.root.exists())


if __name__ == "__main__":
    unittest.main()
