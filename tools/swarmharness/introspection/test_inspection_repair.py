"""Falsifiers for the load-bearing inspection truths: a frozen heartbeat or
old per-plane observation can never present as current verified coverage; a
worker audit PASS/FAIL can never self-certify as editor-verified; absent,
disabled or invalid enrollment can never silently enable, widen or ignore
collection. Also: bounded reads before allocation, empty-file invalid versus
absent, source-shape fuzz, snapshot/cursor/history/partial-scope, privacy,
and retention. All fixtures live under ~/.yggterm/scratchpad (never /tmp);
every test is hermetic — no systemctl, no queue subprocess, no dmesg."""
import json
import os
import shutil
import sys
import time
import unittest
from pathlib import Path
from unittest import mock

from . import adapters, collectors, model, store

NOW = 1_900_000_000.0
SCRATCH = Path.home() / ".yggterm/scratchpad/ygg4-inspect-tests"
GENESIS = "0" * 64


def fixture(name):
    d = SCRATCH / name
    if d.exists():
        shutil.rmtree(d)
    d.mkdir(parents=True)
    return d


def hermetic(fix):
    """Point every collector at the fixture tree and stub host reads."""
    (fix / "dream").mkdir(exist_ok=True)
    (fix / "queue").mkdir(exist_ok=True)
    (fix / "swarm").mkdir(exist_ok=True)
    return mock.patch.multiple(
        collectors,
        HEARTBEAT_STATE=fix / "heartbeat.json",
        DREAM_DIR=fix / "dream",
        QUEUE_STATE=fix / "queue",
        SWARM_STATE=fix / "swarm",
        DREAMER_STATE=fix / "dreamer",
        resolve_queue_source=lambda: (None, "queue cli not resolvable", None),
        _systemctl=lambda argv, timeout: (None, "absent"),
        collect_dmesg=lambda now, limits=None: ([], "absent", "test stub"),
    ), mock.patch.dict(os.environ, {"YGGTERM_HOME": str(fix)})


def write_heartbeat(fix, as_of, planes, findings=None):
    (fix / "heartbeat.json").write_text(json.dumps(
        {"as_of": as_of, "planes": planes, "findings": findings or []}))


def build_chain(review_dir, rid, rows_body):
    from swarmharness import session_review as sr
    rows, prev = [], GENESIS
    for i, body in enumerate(rows_body):
        row = dict(body, seq=i, prev_hash=prev)
        row["record_hash"] = sr._sha(sr._canonical(row))
        rows.append(row)
        prev = row["record_hash"]
    path = Path(review_dir) / (rid + ".jsonl")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(json.dumps(r, sort_keys=True) for r in rows) + "\n")
    return path


def good_chain(review_dir, rid):
    return build_chain(review_dir, rid, [
        {"kind": "review_opened", "review_id": rid, "worker": "fixture-worker",
         "acceptance_criteria": ["criterion-1"], "tier": "full",
         "at": "2026-10-28T00:00:00Z"},
        {"kind": "snapshot_staged", "subject_digest": "d" * 64, "round": 1,
         "completion_claim": {"text": "fixture"}, "at": "2026-10-28T00:01:00Z"},
        {"kind": "editor_verdict", "snapshot_digest": "d" * 64,
         "editor": {"name": "fixture-editor"}, "overall": "COMPLETE",
         "signed_payload": {"overall": "COMPLETE", "findings": []},
         "verification_head": GENESIS, "approval_scope": "full",
         "findings": [], "at": "2026-10-28T00:02:00Z"},
        {"kind": "resolution", "outcome": "COMPLETE", "at": "2026-10-28T00:03:00Z"},
    ])


class FrozenHeartbeat(unittest.TestCase):
    """The parent probe, as executable falsifiers (probe result was FAIL)."""

    def setUp(self):
        self.fix = fixture(self.__class__.__name__)
        self.h1, self.h2 = hermetic(self.fix)

    def tearDown(self):
        shutil.rmtree(self.fix, ignore_errors=True)

    def test_frozen_days_old_heartbeat_is_stale_not_current_verified(self):
        old = NOW - 3 * 86400
        write_heartbeat(self.fix, old, {"p1": {"quality": "verified", "observed_at": old,
                                               "obs": {"grants": [{"grant_units": 5}]}}})
        with self.h1, self.h2:
            frame = adapters.capture(now=NOW, root=self.fix / "store")
        cov = {c["source"]: c for c in frame["coverage"]}["heartbeat:" + os.uname().nodename.split(".")[0]]
        self.assertEqual(cov["quality"], "stale")
        self.assertIn("exceeds ttl", cov["reason"])
        self.assertEqual(model.epoch(cov["observed_at"]), old)  # source as-of, not reader now
        self.assertEqual(model.epoch(cov["received_at"]), NOW)
        funding = [r for r in frame["datasets"]["plans"] if r["kind"] == "funding"]
        self.assertEqual(funding[0]["quality"], "stale")
        self.assertTrue(funding[0]["unknown_reader"])
        check = [r for r in frame["datasets"]["findings"] if r["scope"] == "funding"][0]
        self.assertNotEqual(check["verdict"], "held")

    def test_old_plane_stays_stale_beside_a_current_one(self):
        write_heartbeat(self.fix, NOW, {
            "fresh": {"quality": "verified", "observed_at": NOW - 5},
            "old": {"quality": "verified", "observed_at": NOW - 3600}})
        with self.h1, self.h2:
            frame = adapters.capture(now=NOW, root=self.fix / "store")
        rows = {r["plane"]: r for r in frame["datasets"]["heartbeat"]
                if r.get("row_type") == "plane"}
        self.assertEqual(rows["fresh"]["freshness"], "current")
        self.assertEqual(rows["old"]["freshness"], "stale")
        plans = {r["plane"]: r for r in frame["datasets"]["plans"] if r["kind"] == "funding"}
        self.assertEqual(plans["fresh"]["quality"], "verified")
        self.assertFalse(plans["fresh"]["unknown_reader"])
        self.assertEqual(plans["old"]["quality"], "stale")
        self.assertTrue(plans["old"]["unknown_reader"])

    def test_failed_and_lastverified_planes_never_read_verified(self):
        write_heartbeat(self.fix, NOW, {
            "failed": {"quality": "failed", "observed_at": NOW - 5},
            "lastverified": {"quality": "lastverified", "observed_at": NOW - 5},
            "notime": {"quality": "verified"}})
        with self.h1, self.h2:
            frame = adapters.capture(now=NOW, root=self.fix / "store")
        plans = {r["plane"]: r for r in frame["datasets"]["plans"] if r["kind"] == "funding"}
        for pid in ("failed", "lastverified", "notime"):
            self.assertEqual(plans[pid]["quality"], "unknown", pid)
            self.assertTrue(plans[pid]["unknown_reader"], pid)
        for pid in ("failed", "lastverified"):
            self.assertNotEqual(plans[pid]["reader_quality"], "verified", pid)
        check = [r for r in frame["datasets"]["findings"] if r["scope"] == "funding"][0]
        self.assertEqual(check["verdict"], "unknown")


class EditorVerification(unittest.TestCase):
    """A worker audit PASS/FAIL is a claim, never editor-verified."""

    def setUp(self):
        self.fix = fixture(self.__class__.__name__)
        self.h1, self.h2 = hermetic(self.fix)

    def tearDown(self):
        shutil.rmtree(self.fix, ignore_errors=True)

    def _audit(self, fix, rows):
        d = fix / "dreamer"
        d.mkdir(exist_ok=True)
        (d / "audit-ledger.jsonl").write_text(
            "".join(json.dumps(r) + "\n" for r in rows))

    def test_audit_pass_without_receipt_stays_unverified(self):
        self._audit(self.fix, [{"id": "task-1", "repair_ref": "commit-abc",
                                "verdict": "PASS", "audited_at": NOW}])
        with self.h1, self.h2:
            frame = adapters.capture(now=NOW, root=self.fix / "store")
        repair = frame["datasets"]["repairs"][0]
        self.assertEqual(repair["verdict"], "PASS")  # recorded claim, kept
        self.assertEqual(repair["verification"], "unverified")
        self.assertIn("never self-certifies", repair["verification_reason"])

    def test_valid_chain_without_control_key_is_not_editor_verified(self):
        rid = "rev-fixture-1"
        good_chain(self.fix / "reviews", rid)
        state, why = adapters._receipt_state(rid, self.fix / "reviews",
                                             key_path=self.fix / "no-such-key")
        self.assertEqual(state, "unverified")
        self.assertIn("provenance not verifiable", why)

    def test_tampered_chain_is_rejected_not_appeased(self):
        rid = "rev-fixture-2"
        path = good_chain(self.fix / "reviews", rid)
        lines = path.read_text().splitlines()
        row = json.loads(lines[2])
        row["overall"] = "OWNER_STOPPED"  # tamper without rehashing
        lines[2] = json.dumps(row, sort_keys=True)
        path.write_text("\n".join(lines) + "\n")
        state, why = adapters._receipt_state(rid, self.fix / "reviews",
                                             key_path=self.fix / "no-such-key")
        self.assertEqual(state, "unverified")
        self.assertIn("corrupt", why)

    def test_incomplete_chain_has_no_complete_resolution(self):
        rid = "rev-fixture-3"
        build_chain(self.fix / "reviews", rid, [
            {"kind": "review_opened", "review_id": rid, "worker": "w",
             "acceptance_criteria": ["c"], "tier": "full", "at": "2026-10-28T00:00:00Z"}])
        state, why = adapters._receipt_state(rid, self.fix / "reviews",
                                             key_path=self.fix / "no-such-key")
        self.assertEqual(state, "unverified")
        self.assertIn("COMPLETE resolution", why)


class EnrollmentModes(unittest.TestCase):
    def setUp(self):
        self.fix = fixture(self.__class__.__name__)
        self.h1, self.h2 = hermetic(self.fix)

    def tearDown(self):
        shutil.rmtree(self.fix, ignore_errors=True)

    def _load(self, result=None, raising=False):
        if raising:
            return mock.patch("swarmharness.swarmconfig.load",
                              side_effect=RuntimeError("config layer broken"))
        return mock.patch("swarmharness.swarmconfig.load", return_value=result)

    def _cfg(self, introspection=None, mode="active", faults=()):
        cfg = {"fleet": {"hosts": ["local", "dev"]}}
        if introspection is not None:
            cfg["introspection"] = introspection
        return {"mode": mode, "config": cfg, "faults": list(faults)}

    def test_absent_subtree_is_standing_and_marked_not_configured_complete(self):
        with self.h1, self.h2, self._load(self._cfg()):
            frame = adapters.capture(now=NOW, root=self.fix / "store")
        self.assertTrue(all(c.get("collection_mode") == "standing"
                            for c in frame["coverage"]))
        ok_rows = [c for c in frame["coverage"] if c["quality"] == "verified"]
        self.assertTrue(ok_rows)
        for c in ok_rows:
            self.assertIn("not configured-complete", c["reason"])

    def test_explicit_disable_fails_closed_without_standing_substitution(self):
        with self.h1, self.h2, self._load(self._cfg({"enabled": False})):
            frame = adapters.capture(now=NOW, root=self.fix / "store")
        self.assertEqual(frame["datasets"]["summary"][0]["collection_mode"], "disabled")
        for name, rows in frame["datasets"].items():
            if name not in ("findings", "summary"):
                self.assertEqual(rows, [], name)
        cov = frame["coverage"][0]
        self.assertEqual(cov["quality"], "unavailable")
        self.assertIn("disabled by enrollment", cov["reason"])

    def test_inspection_faults_fail_closed(self):
        with self.h1, self.h2, self._load(self._cfg(
                mode="containment_only",
                faults=[{"code": "inspection_schema", "detail": "introspection.sources bad"}])):
            frame = adapters.capture(now=NOW, root=self.fix / "store")
        self.assertEqual(frame["datasets"]["summary"][0]["collection_mode"], "invalid")
        self.assertIn("fail closed", frame["coverage"][0]["reason"])
        self.assertEqual(frame["datasets"]["dreams"], [])

    def test_broken_config_layer_fails_closed(self):
        with self.h1, self.h2, self._load(raising=True):
            frame = adapters.capture(now=NOW, root=self.fix / "store")
        self.assertEqual(frame["datasets"]["summary"][0]["collection_mode"], "invalid")

    def test_configured_sources_control_collectors_and_scope(self):
        sub = {"version": 1, "enabled": True, "sources": {
            "svc": {"kind": "services", "host": "local", "enabled": True,
                    "ttl_seconds": 60, "units": ["enrolled-example.service"]},
            "remote": {"kind": "services", "host": "oc", "enabled": True,
                       "ttl_seconds": 60, "units": ["x.service"]},
            "rev": {"kind": "reviews", "host": "local", "enabled": True,
                    "ttl_seconds": 60, "path": "/unused"},
            "off": {"kind": "heartbeat", "host": "local", "enabled": False,
                    "ttl_seconds": 60, "path": "/unused"}}}
        seen_units = []

        def fake_services(units, now, limits=None):
            seen_units.append(list(units or []))
            return [], "absent", {"expected": units or [], "observed": [],
                                  "expected_not_found": units or []}

        with self.h1, self.h2, self._load(self._cfg(sub)):
            with mock.patch.object(collectors, "collect_services", fake_services):
                frame = adapters.capture(now=NOW, root=self.fix / "store")
        self.assertEqual(frame["datasets"]["summary"][0]["collection_mode"], "configured")
        self.assertEqual(seen_units, [["enrolled-example.service"]])
        local = os.uname().nodename.split(".")[0]
        cov = {c["source"]: c for c in frame["coverage"]}
        self.assertEqual(cov["services:svc:" + local]["quality"], "unavailable")  # absent roster
        self.assertIn("not silently widened", cov["services:remote:" + local]["reason"])
        self.assertIn("not implemented", cov["reviews:rev:" + local]["reason"])
        self.assertIn("disabled; not collected", cov["heartbeat:off:" + local]["reason"])
        # standing-only sources are absent from a configured capture
        for name in frame["coverage"]:
            self.assertFalse(name["source"].startswith(("dreamer-ledgers:", "kernel-ring:",
                                                        "dream-state:")), name["source"])


class BoundedReads(unittest.TestCase):
    def setUp(self):
        self.fix = fixture(self.__class__.__name__)

    def tearDown(self):
        shutil.rmtree(self.fix, ignore_errors=True)

    def test_empty_file_is_invalid_not_absent(self):
        p = self.fix / "empty.json"
        p.write_text("")
        self.assertEqual(collectors._state(p)[1], "invalid")
        p.write_text("   \n")
        self.assertEqual(collectors._state(p)[1], "invalid")
        self.assertEqual(collectors.read_jsonl(p)[1], "invalid")
        self.assertEqual(collectors._state(self.fix / "missing.json")[1], "absent")
        self.assertEqual(collectors._state(self.fix)[1], "unreadable")

    def test_oversized_file_is_refused_without_explosive_read(self):
        p = self.fix / "big.json"
        with p.open("wb") as f:
            f.seek(64 * 1024 + 10)
            f.write(b"x")
        value, state, detail = collectors._state(p, max_bytes=4096)
        self.assertEqual(state, "invalid")
        self.assertIn("over read limit", detail)
        self.assertIsNone(value)

    def test_capture_survives_empty_and_garbage_sources(self):
        h1, h2 = hermetic(self.fix)
        (self.fix / "heartbeat.json").write_text("")  # empty, not absent
        with h1, h2:
            frame = adapters.capture(now=NOW, root=self.fix / "store")
        cov = [c for c in frame["coverage"] if c["source"].startswith("heartbeat:")]
        self.assertEqual(cov[0]["quality"], "unknown")

    def test_source_shape_fuzz_never_crashes(self):
        h1, h2 = hermetic(self.fix)
        write_heartbeat(self.fix, NOW, {
            "junk": "not-a-dict", "weird": {"quality": 7, "observed_at": "soon",
                                            "obs": {"grants": ["x", 3, None]}}},
            findings="not-a-list")
        with h1, h2:
            frame = adapters.capture(now=NOW, root=self.fix / "store")  # must not raise
        for repair in adapters._repairs_from_audit(
                [None, 5, {"repair_ref": ["odd"]}, {"id": "t", "repair_ref": "r",
                                                    "verdict": {"deep": 1}}], NOW):
            self.assertEqual(repair["verification"], "unverified")
        model.queue_records([{"id": None}, {"id": "t2", "attempts": "x"}],
                            [{"ev": "dispatch"}, "junk", {}], {}, "q", "h", NOW)


class SnapshotStoreAndQueries(unittest.TestCase):
    def setUp(self):
        self.fix = fixture(self.__class__.__name__)
        self.root = self.fix / "store"

    def tearDown(self):
        shutil.rmtree(self.fix, ignore_errors=True)

    def _frame(self, dreams=(("work-1",),)):
        cov = [model.coverage("queue-example", "worker-a", "queue",
                              observed_at=model.iso(NOW), received_at=model.iso(NOW),
                              ttl=60, revision=None, collection_mode="standing")]
        rows = [{"id": "work:" + d[0]} for d in dreams]
        return model.frame("a" * 64, model.iso(NOW), cov,
                           {"dreams": rows, "attempts": []})

    def test_snapshot_write_once_and_tamper_detection(self):
        f1 = self._frame()
        sid, created = store.save_snapshot(f1, root=self.root)
        self.assertTrue(created)
        f2 = self._frame()
        sid2, created2 = store.save_snapshot(f2, root=self.root)
        self.assertEqual((sid, created2), (sid, False))  # idempotent, no overwrite
        self.assertEqual(store.load_snapshot(sid, root=self.root)["snapshot_id"], sid)
        path = store.snapshots_dir(self.root) / (sid + ".json")
        tampered = json.loads(path.read_text())
        tampered["datasets"]["dreams"].append({"id": "smuggled"})
        path.write_text(json.dumps(tampered))
        with self.assertRaisesRegex(ValueError, "digest"):
            store.load_snapshot(sid, root=self.root)

    def test_prune_enforces_days_and_byte_cap(self):
        old = self._frame()
        old["as_of"] = model.iso(NOW - 40 * 86400)
        old = model.frame(old["config_hash"], old["as_of"], old["coverage"], old["datasets"])
        store.save_snapshot(old, root=self.root)
        old_path = next(store.snapshots_dir(self.root).glob("*.json"))
        os.utime(old_path, (NOW - 40 * 86400, NOW - 40 * 86400))
        new = self._frame()
        store.save_snapshot(new, root=self.root)
        new_path = store.snapshots_dir(self.root) / (new["snapshot_id"] + ".json")
        os.utime(new_path, (NOW, NOW))  # fixture NOW sits ahead of the wall clock
        removed = store.prune(root=self.root, now=NOW, retention={"raw_days": 31})
        self.assertEqual(removed["snapshots"], 1)
        self.assertIsNone(store.load_snapshot(old["snapshot_id"], root=self.root))
        removed2 = store.prune(root=self.root, now=NOW,
                               retention={"raw_days": 31, "max_store_bytes": 1})
        self.assertGreaterEqual(removed2["snapshots"], 1)

    def test_events_dedupe_refuse_malformed_and_filter(self):
        rows = [{"id": "event:dmesg:1", "summary": "oom", "subsystem": "kernel", "at": NOW},
                {"summary": "no id"}]
        out = store.append_events(rows, root=self.root, now=NOW)
        self.assertEqual(out["appended"], 1)
        out2 = store.append_events([rows[0]], root=self.root, now=NOW)
        self.assertEqual(out2["appended"], 0)
        self.assertEqual(store.read_events(root=self.root, subsystem=["kernel"])[0]["id"],
                         "event:dmesg:1")
        self.assertEqual(store.read_events(root=self.root, since=NOW + 10), [])

    def test_partial_scope_reports_missing_enrolled_source(self):
        snap = self._frame()
        result, code = model.query(snap, ("dream", "ls"), {"host": "nowhere"}, NOW)
        self.assertEqual(code, 3)
        self.assertIn("NO_ENROLLED_SOURCE", [f["code"] for f in result["findings"]])

    def test_privacy_projection_refuses_prompt_and_credential(self):
        repair = {"id": "repair:1", "ref": "Bearer fixture-secret-token-123",
                  "verdict": "PASS", "verification": "unverified"}
        self.assertNotIn("fixture-secret-token", model.text(repair["ref"]))
        cov = [model.coverage("queue-example", "worker-a", "queue",
                              observed_at=model.iso(NOW), received_at=model.iso(NOW),
                              ttl=60)]
        snap = model.frame("a" * 64, model.iso(NOW), cov,
                           {"dreams": [], "attempts": [], "repairs": [repair]})
        with self.assertRaisesRegex(ValueError, "credential"):
            model.validate_frame(snap)


class KernelLayout(unittest.TestCase):
    def test_package_imports_from_staged_kernel_without_checkout(self):
        fix = fixture("KernelLayout")
        try:
            staged = fix / "kernel" / "swarmharness"
            shutil.copytree(Path(collectors.__file__).resolve().parent,
                            staged / "introspection")
            shutil.copy(Path(collectors.__file__).resolve().parents[1] / "swarm_ledger.py",
                        staged / "swarm_ledger.py")
            sys.path.insert(0, str(fix / "kernel"))
            try:
                for mod in list(sys.modules):
                    if mod.startswith("swarmharness"):
                        del sys.modules[mod]
                import swarmharness.introspection.collectors as staged_collectors
                rows, state, detail = staged_collectors.collect_commitments(str(fix), now=NOW)
                self.assertIn(state, ("ok", "absent", "conflicting"))
                cands = staged_collectors._ledger_path_dirs()
                self.assertTrue(any((c / "swarm_ledger.py").exists() for c in cands))
            finally:
                sys.path.remove(str(fix / "kernel"))
                for mod in list(sys.modules):
                    if mod.startswith("swarmharness"):
                        del sys.modules[mod]
        finally:
            shutil.rmtree(fix, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
