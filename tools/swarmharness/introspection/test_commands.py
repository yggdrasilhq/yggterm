#!/usr/bin/env python3
"""Contract tests for the one command tree (commands.json + commands.py).

The IR owns grammar, components, roles and validation. These tests pin the
contract's structure and the loader's refusals; behavioral parity with the
native frontend lives in crates/ygg-cli/tests.
"""
import copy
import importlib
import json
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import commands


class ComponentDeclarations(unittest.TestCase):
    def test_every_legacy_command_names_a_declared_component(self):
        for path, node in commands.TREE.items():
            if node.handler != "legacy":
                continue
            self.assertTrue(node.component, f"legacy command {path} has no component")
            self.assertIn(node.component, commands.COMPONENTS, path)
            declared = commands.COMPONENTS[node.component]
            roster = {declared["runner"], *declared.get("aliases", [])}
            target = node.legacy[0].lstrip("@")
            self.assertIn(target, roster, f"{path} target {target} not owned by {node.component}")

    def test_components_declare_data_ownership(self):
        for name, declared in commands.COMPONENTS.items():
            self.assertTrue(declared.get("owns"), f"component {name} owns nothing (undeclared data)")
            self.assertTrue(declared.get("resolution"), f"component {name} has no resolution chain")
            if declared.get("interpreter"):
                self.assertNotIn(" ", declared["interpreter"])

    def test_memory_component_is_owner_managed_with_env_chain(self):
        memory = commands.COMPONENTS["memory"]
        self.assertTrue(memory["owner_managed"])
        self.assertEqual(memory["resolution"][0], "env:YGG_MEMORY_BIN")

    def test_ynpm_component_never_resolves_through_a_ygg_shim(self):
        chain = commands.COMPONENTS["ynpm"]["resolution"]
        self.assertTrue(all("ygg " not in s and s != "path:ygg" for s in chain))
        self.assertIn("env:YGG_YNPM_BIN", chain)


class ContractSurface(unittest.TestCase):
    def test_primary_doors_exist(self):
        for path in (("ynpm",), ("skills",), ("memory",), ("ci",), ("init",), ("doctor",),
                     ("dream",), ("service",), ("schedule",), ("swarm",), ("board",),
                     ("payload",), ("payload", "manifest"), ("payload", "verify"),
                     ("payload", "install"), ("daemon",), ("agent",), ("work",),
                     ("relay",), ("gui",)):
            self.assertIn(path, commands.TREE, path)

    def test_ynpm_and_skills_are_declared_passthrough(self):
        for path in (("ynpm",), ("skills",)):
            node = commands.TREE[path]
            self.assertEqual(node.handler, "legacy")
            self.assertEqual(node.component, "ynpm")
            self.assertFalse(node.strict)

    def test_planned_families_refuse_with_explicit_role(self):
        for name in ("daemon", "agent", "work", "relay", "gui"):
            node = commands.TREE[(name,)]
            self.assertEqual(node.role, "planned")
            with self.assertRaises(commands.PlannedFamily):
                commands.parse([name, "anything"])

    def test_payload_leaves_are_native_handlers(self):
        self.assertEqual(commands.TREE[("payload", "manifest")].native, "payload_manifest")
        self.assertEqual(commands.TREE[("payload", "verify")].native, "payload_verify")
        self.assertEqual(commands.TREE[("payload", "install")].native, "payload_install")

    def test_no_generation_claims_a_wired_adapter_yet(self):
        self.assertEqual(commands.WIRED_ADAPTERS, {})


class DeclarativeValidation(unittest.TestCase):
    def test_limit_range_is_declared_in_the_shared_option_set(self):
        option_set = json.loads((HERE / "commands.json").read_text())["option_sets"]["common"]
        ranges = {tuple(o["flags"]): o.get("int_range") for o in option_set if "flags" in o}
        self.assertEqual(ranges[("--limit",)], [1, 1000])

    def test_dream_open_selection_is_declared_not_hardcoded(self):
        record = json.loads((HERE / "commands.json").read_text())
        entry = next(r for r in record["commands"] if r["path"] == ["dream", "open"])
        self.assertEqual(entry["require_one_of"], ["id", "pick"])
        self.assertEqual(entry["conflicts"], [["id", "pick"]])

    def test_completion_shell_is_declared(self):
        record = json.loads((HERE / "commands.json").read_text())
        entry = next(r for r in record["commands"] if r["path"] == ["completion"])
        self.assertEqual(entry["operand_choices"], ["bash"])

    def test_argparse_matrix_matches_the_router_tests(self):
        for args in (("dream", "topup", "--typo"), ("dream", "ls", "--state", "made-up"),
                     ("dream", "inspect"), ("dream", "open"),
                     ("dream", "open", "dream-42", "--pick"),
                     ("swarm", "pressure", "--lim", "2"),
                     ("service", "ps", "--limit", "0"), ("service", "ps", "--limit", "bad"),
                     ("payload", "install"), ("completion", "zsh")):
            with self.assertRaises(SystemExit) as ctx:
                commands.parse(list(args))
            self.assertEqual(ctx.exception.code, 2, args)

    def test_valid_selections_parse(self):
        self.assertEqual(commands.parse(["dream", "open", "dream-42"])[0], ("dream", "open"))
        self.assertEqual(commands.parse(["dream", "open", "--pick"])[0], ("dream", "open"))
        path, args, _ = commands.parse(["service", "ps", "--limit", "7", "--json"])
        self.assertEqual((path, args.limit, args.json), (("service", "ps"), 7, True))
        path, _, rest = commands.parse(["ynpm", "install", "example-pkg", "--future-flag"])
        self.assertEqual((path, rest), (("ynpm",), ["install", "example-pkg", "--future-flag"]))


class LoaderRefusals(unittest.TestCase):
    def _load_document(self):
        return json.loads((HERE / "commands.json").read_text())

    def _refused(self, document):
        """Swap the contract bytes, reload the loader, capture the refusal."""
        import importlib
        original = (HERE / "commands.json").read_bytes()
        (HERE / "commands.json").write_text(json.dumps(document))
        try:
            importlib.reload(commands)
        except ValueError as exc:
            commands_module_error = str(exc)
        else:
            commands_module_error = None
        finally:
            (HERE / "commands.json").write_bytes(original)
            importlib.reload(commands)
        self.assertIsNotNone(commands_module_error)
        return commands_module_error

    def test_undeclared_component_is_refused(self):
        document = self._load_document()
        entry = next(r for r in document["commands"] if r["path"] == ["ci"])
        entry["component"] = "unclaimed_plane"
        self.assertIn("undeclared component", self._refused(document) or "")

    def test_unowned_target_is_refused(self):
        document = self._load_document()
        entry = next(r for r in document["commands"] if r["path"] == ["ci"])
        entry["legacy"] = ["ygg-panic.py"]
        self.assertIn("not owned by its component", self._refused(document) or "")

    def test_unknown_role_is_refused(self):
        document = self._load_document()
        document["commands"].append({"path": ["ghost"], "summary": "x", "role": "weird", "options": []})
        self.assertIn("unsupported command role", self._refused(document) or "")

    def test_wired_adapter_must_name_a_family(self):
        document = self._load_document()
        document["wired_adapters"] = {"not-a-family": {"adapter": "example"}}
        self.assertIn("unknown family", self._refused(document) or "")


class GenerationIdentity(unittest.TestCase):
    def test_contract_hash_is_the_file_bytes_sha256_and_stable(self):
        import hashlib
        raw = (HERE / "commands.json").read_bytes()
        self.assertEqual(commands.CONTRACT_HASH, hashlib.sha256(raw).hexdigest())
        self.assertEqual(commands.CONTRACT_HASH, commands.CONTRACT_HASH)

    def test_document_header_names_the_contract_and_supported_schema(self):
        document = json.loads((HERE / "commands.json").read_text())
        self.assertEqual(document["schema_version"], 1)
        self.assertEqual(document["contract"], "ygg4-cli")
        self.assertEqual(document["frontend"]["supported_schema"], [1])


if __name__ == "__main__":
    unittest.main()
