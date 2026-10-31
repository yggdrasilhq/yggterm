"""Hermetic local-Linux owner witnesses and metadata-only GC controls."""
import concurrent.futures
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
SCRIPT = Path(__file__).resolve().parents[1] / "ygg_row.py"
spec = importlib.util.spec_from_file_location("row_owner_subject", SCRIPT)
row = importlib.util.module_from_spec(spec)
spec.loader.exec_module(row)


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux process binding contract")
class OwnerTests(unittest.TestCase):
    def setUp(self):
        scratch = Path.home() / ".yggterm/scratchpad/row-owner-tests"
        scratch.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=scratch)
        self.root = Path(self.temp.name)
        self.env = dict(os.environ, HOME=str(self.root), YGGTERM_HOME=str(self.root / "store"),
                        YGGTERM_SESSION_ID="test-supervisor", PYTHONDONTWRITEBYTECODE="1")
        self.environment = patch.dict(os.environ, self.env)
        self.environment.start()
        self.store = row.rows_dir()
        self.claims = self.store / "claims"
        row.write_json(self.store / "campaigns.json", {"7": {"name": "invented"}})
        self.processes = []

    def tearDown(self):
        for p in self.processes:
            if p.poll() is None:
                p.terminate()
            p.wait(timeout=5)
        self.environment.stop()
        self.temp.cleanup()

    def process(self):
        p = subprocess.Popen([sys.executable, "-B", "-c", "import time; time.sleep(120)"], env=self.env)
        self.processes.append(p)
        return p

    def cli(self, *args, ok=True, env=None):
        r = subprocess.run([sys.executable, "-B", str(SCRIPT), *args], env=env or self.env,
                           capture_output=True, text=True, timeout=15)
        if ok:
            self.assertEqual(r.returncode, 0, r.stderr)
        else:
            self.assertNotEqual(r.returncode, 0)
        return r

    def claim(self, pid=None, sid=None):
        args = ["claim", "--campaign", "7"]
        if pid is not None:
            args += ["--pid", str(pid)]
        self.cli(*args, env=dict(self.env, YGGTERM_SESSION_ID=sid) if sid else None)
        return self.claims / "7.1.json"

    def expired_dead(self):
        p = self.process()
        path = self.claim(p.pid)
        p.terminate()
        p.wait(timeout=5)
        c = row.read_record(path)
        c["heartbeat"] = time.time() - 90000
        row.write_json(path, c)
        self.assertEqual(row.process_liveness(c)["state"], "DEAD")
        return path, c

    def gc(self):
        with contextlib.redirect_stdout(io.StringIO()):
            row.cmd_gc(SimpleNamespace(dry_run=False))

    def test_supervisor_alive_after_claim_command_exits(self):
        p = self.process()
        path = self.claim(p.pid)
        c = row.read_record(path)
        self.assertEqual(c["pid"], p.pid)
        self.assertIsNone(p.poll())
        self.assertEqual(row.process_liveness(c)["state"], "ALIVE")
        self.assertEqual(json.loads(self.cli("list", "--json").stdout)[0]["process_liveness"]["state"], "ALIVE")

    def test_default_unbound_and_heartbeat_never_adopts_cli(self):
        path = self.claim()
        c = row.read_record(path)
        self.assertIsNone(c["pid"])
        self.cli("heartbeat", "7.1")
        self.assertEqual(row.process_liveness(row.read_record(path))["state"], "UNBOUND")
        self.assertIsNone(row.read_record(path)["pid"])

    def test_heartbeat_preserves_binding_and_explicit_rebind(self):
        p, q = self.process(), self.process()
        path = self.claim(p.pid)
        before = row.read_record(path)
        self.cli("heartbeat", "7.1", "--model", "invented")
        after = row.read_record(path)
        self.assertEqual(after["process_owner"], before["process_owner"])
        self.assertEqual(after["pid"], p.pid)
        self.cli("heartbeat", "7.1", "--pid", str(q.pid))
        self.assertEqual(row.read_record(path)["pid"], q.pid)

    def test_invalid_pid_rejected_for_all_binding_verbs(self):
        self.claim()
        original = (self.claims / "7.1.json").read_bytes()
        for pid in ("0", "-1", "999999999"):
            self.cli("heartbeat", "7.1", "--pid", pid, ok=False)
            self.cli("claim", "--campaign", "7", "--id", "7.2", "--pid", pid, ok=False)
            self.cli("register-bridge", "--name", "invented", "--harness", "test", "--pid", pid, ok=False)
        self.assertEqual((self.claims / "7.1.json").read_bytes(), original)
        self.assertFalse((self.claims / "7.2.json").exists())

    def test_idempotent_claim_does_not_silently_ignore_requested_binding(self):
        p, q = self.process(), self.process()
        path = self.claim(p.pid)
        before = path.read_bytes()
        self.cli("claim", "--campaign", "7", "--pid", str(p.pid))
        self.assertEqual(path.read_bytes(), before)
        self.cli("claim", "--campaign", "7", "--pid", str(q.pid), ok=False)
        self.cli("claim", "--campaign", "7", "--pid", "-1", ok=False)
        self.assertEqual(path.read_bytes(), before)

    def test_foreign_reused_legacy_inaccessible_and_unsupported_unknown(self):
        p = self.process()
        c = row.read_record(self.claim(p.pid))
        with patch.object(row, "process_sample", side_effect=PermissionError()):
            self.assertEqual(row.process_liveness(c)["state"], "UNKNOWN")
            with self.assertRaises(SystemExit):
                row.bind_process(p.pid)
        with patch.object(row.sys, "platform", "unsupported"):
            self.assertEqual(row.process_liveness(c)["state"], "UNKNOWN")
            with self.assertRaises(SystemExit):
                row.bind_process(p.pid)
        with patch.object(row, "process_sample", return_value={"uid": os.getuid() + 1, "state": "S", "starttime": "1"}):
            with self.assertRaises(SystemExit):
                row.bind_process(p.pid)
        for key in ("host", "boot_id", "uid", "platform", "pid_namespace"):
            other = json.loads(json.dumps(c))
            other["process_owner"][key] = "foreign"
            self.assertEqual(row.process_liveness(other)["state"], "UNKNOWN")
        c["process_owner"]["starttime"] += "0"
        self.assertEqual(row.process_liveness(c)["state"], "REUSED")
        c.pop("process_owner")
        self.assertEqual(row.process_liveness(c)["state"], "UNKNOWN")

    def test_bridge_binding_defaults_and_pin_preservation(self):
        args = ("register-bridge", "--name", "invented", "--harness", "test")
        self.cli(*args)
        path = self.store / "bridges/invented.json"
        self.assertIsNone(row.read_record(path)["pid"])
        c = row.read_record(path)
        c["owner_pinned"] = True
        row.write_json(path, c)
        p = self.process()
        self.cli(*args, "--pid", str(p.pid))
        self.assertTrue(row.read_record(path)["owner_pinned"])
        self.assertEqual(row.process_liveness(row.read_record(path))["state"], "ALIVE")
        self.cli(*args)
        self.assertEqual(row.read_record(path)["pid"], p.pid)
        self.assertTrue(row.read_record(path)["owner_pinned"])

    def test_expired_live_is_retained_and_not_rendered_reaped(self):
        p = self.process()
        path = self.claim(p.pid)
        c = row.read_record(path)
        c["heartbeat"] = time.time() - 90000
        row.write_json(path, c)
        self.gc()
        self.assertTrue(path.exists())
        output = self.cli("list").stdout
        self.assertIn("EXPIRED", output)
        self.assertIn("ALIVE", output)
        self.assertNotIn("REAPED", output)

    def test_hidden_live_target_is_unknown_and_gc_retains_it(self):
        p = self.process()
        path = self.claim(p.pid)
        record = row.read_record(path)
        record["heartbeat"] = time.time() - 90000
        row.write_json(path, record)
        sample = row.process_sample
        def hidden(pid):
            if pid == p.pid:
                raise FileNotFoundError()
            return sample(pid)
        with patch.object(row, "process_sample", side_effect=hidden):
            self.assertEqual(row.process_liveness(record)["state"], "UNKNOWN")
            self.gc()
        self.assertTrue(path.exists())
        self.assertIsNone(p.poll())

    def test_procfs_namespace_mismatch_refuses_binding_and_liveness(self):
        p = self.process()
        record = row.read_record(self.claim(p.pid))
        real = row.os.readlink
        with patch.object(row.os, "readlink", side_effect=lambda path: "foreign" if str(path) == "/proc/self" else real(path)):
            with self.assertRaises(SystemExit):
                row.bind_process(p.pid)
            self.assertEqual(row.process_liveness(record)["state"], "UNKNOWN")

    def test_oversized_numbers_are_unknown_in_all_views_and_gc(self):
        path, original = self.expired_dead()
        for key in ("heartbeat", "ttl_hours"):
            for value in (10**400, -(10**400), float("inf"), float("nan")):
                row.write_json(path, {**original, key: value})
                self.assertEqual(row.freshness(row.read_record(path)), "UNKNOWN")
                self.cli("list")
                self.cli("list", "--json")
                self.gc()
                self.assertTrue(path.exists())

    def test_oversized_pid_never_reaches_kernel_conversion(self):
        path, original = self.expired_dead()
        for pid in (2**31, 10**400):
            row.write_json(path, {**original, "pid": pid})
            with patch.object(row.os, "pidfd_open", side_effect=AssertionError("invalid PID reached kernel")):
                self.assertEqual(row.process_liveness(row.read_record(path))["state"], "UNKNOWN")
                self.gc()
            self.cli("list")
            self.cli("list", "--json")
            self.assertTrue(path.exists())
            self.cli("heartbeat", "7.1", "--pid", str(pid), ok=False)

    def test_gc_only_expired_known_dead_explicitly_unpinned(self):
        path, original = self.expired_dead()
        for directory in (self.claims, self.store / "bridges"):
            target = directory / "7.1.json"
            for change in ({"owner_pinned": True}, {"pinned": True}, {"pin": {"owner": True}},
                           {"owner_pinned": "false"}, {"heartbeat": time.time()},
                           {"pid": None, "process_owner": None}, {"process_owner": None},
                           {"heartbeat": "bad"}, {"ttl_hours": -1}):
                row.write_json(target, {**original, **change})
                self.gc()
                self.assertTrue(target.exists(), change)
            unknown_pin = dict(original)
            unknown_pin.pop("owner_pinned")
            row.write_json(target, unknown_pin)
            self.gc()
            self.assertTrue(target.exists())
            row.write_json(target, original)
            self.cli("gc", "--dry-run")
            self.assertTrue(target.exists())
            self.gc()
            self.assertFalse(target.exists())

    def test_malformed_records_visible_typed_unknown_and_retained(self):
        values = [None, [], ["future"], "opaque", 4, False]
        for directory in (self.claims, self.store / "bridges"):
            for i, value in enumerate(values):
                row.write_json(directory / f"7.{i}.json", value)
            (directory / "7.9.json").write_text("invalid json")
        entries = json.loads(self.cli("list", "--json").stdout)
        self.assertEqual(len(entries), 14)
        self.assertTrue(all(e["process_liveness"]["state"] == "UNKNOWN" for e in entries))
        self.cli("list")
        self.assertEqual(row.known_hosts(self.store), [])
        before = sorted(str(p) for p in self.store.glob("*/*.json"))
        self.gc()
        self.assertEqual(sorted(str(p) for p in self.store.glob("*/*.json")), before)

    def test_gc_revalidates_heartbeat_rebind_and_pin_after_candidate_scan(self):
        path, original = self.expired_dead()
        live = self.process()
        for change in ({"heartbeat": time.time()}, row.bind_process(live.pid), {"owner_pinned": True}):
            row.write_json(path, original)
            # Deterministic race: another cooperating writer changes the record
            # after the candidate read, before GC acquires its mutation fence.
            @contextlib.contextmanager
            def racing_fence(store):
                with real_fence(store):
                    row.write_json(path, {**original, **change})
                    yield
            real_fence = row.mutation_fence
            with patch.object(row, "mutation_fence", racing_fence):
                self.gc()
            self.assertTrue(path.exists(), change)

    def test_concurrent_heartbeat_and_gc_share_mutation_fence(self):
        path, _ = self.expired_dead()
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            with row.mutation_fence(self.store):
                hb = pool.submit(self.cli, "heartbeat", "7.1")
                gc = pool.submit(self.cli, "gc")
                # Both subprocesses are fenced out while the current owner
                # publishes freshness. The fence releases before joining.
                current = row.read_record(path)
                current["heartbeat"] = time.time()
                row.write_json(path, current)
            hb.result(timeout=20)
            gc.result(timeout=20)
        self.assertTrue(path.exists())
        self.assertEqual(row.freshness(row.read_record(path)), "FRESH")


if __name__ == "__main__":
    unittest.main()
