"""Falsifiers for the load-bearing inspection truths: a frozen heartbeat or
old per-plane observation can never present as current verified coverage; a
worker audit PASS/FAIL can never self-certify as editor-verified; absent,
disabled or invalid enrollment can never silently enable, widen or ignore
collection. Also: bounded reads before allocation, empty-file invalid versus
absent, source-shape fuzz, snapshot/cursor/history/partial-scope, privacy,
and retention. All fixtures live under ~/.yggterm/scratchpad (never /tmp);
every test is hermetic — no systemctl, no queue subprocess, no dmesg."""
import calendar
import contextlib
import io
import json
import os
import shutil
import sys
import threading
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
        COMPOSER_STATE=fix / "composer",
        REVIEWS_DIR=fix / "reviews",
        MEMORY_LOCK_DIR=fix / "memlock",
        resolve_queue_source=lambda: (None, "queue cli not resolvable", None),
        _systemctl=lambda argv, timeout: (None, "absent"),
        collect_dmesg=lambda now, limits=None: ([], "absent", "test stub"),
    ), mock.patch.dict(os.environ, {"YGGTERM_HOME": str(fix),
                                    "YGG_INSPECT_REVIEWS": str(fix / "reviews"),
                                    "YGG_MEMORY_LOCK_DIR": str(fix / "memlock")})


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
        # An empty fixture tree has NO verified sources: every previously
        # reader-clock-"verified" row now carries its (missing) source clock
        # and is demoted with the exact reason — verified on a source whose
        # observation time is unknown was the Law-1 violation.
        ok_rows = [c for c in frame["coverage"] if c["quality"] == "verified"]
        self.assertEqual(ok_rows, [])
        for c in frame["coverage"]:
            self.assertNotEqual(c.get("observed_at"), model.iso(NOW),
                                c["source"] + " must not carry the reader clock")
        # A file-backed source with its own fresh as_of is verified from the
        # SOURCE clock; an old as_of is stale even though the read succeeded.
        dream = self.fix / "dream" / "state.json"
        dream.write_text(json.dumps({"as_of": model.iso(NOW - 30),
                                     "counts": {"queued": 1}}))
        os.utime(dream, (NOW - 30, NOW - 30))
        with self.h1, self.h2, self._load(self._cfg()):
            frame = adapters.capture(now=NOW, root=self.fix / "store")
        cov = {c["source"]: c for c in frame["coverage"]}
        fresh = cov["dream-state:" + os.uname().nodename.split(".")[0]]
        self.assertEqual(fresh["quality"], "verified")
        self.assertEqual(model.epoch(fresh["observed_at"]), NOW - 30)
        dream.write_text(json.dumps({"as_of": model.iso(NOW - 10 * 86400)}))
        with self.h1, self.h2, self._load(self._cfg()):
            frame = adapters.capture(now=NOW, root=self.fix / "store")
        cov = {c["source"]: c for c in frame["coverage"]}
        stale = cov["dream-state:" + os.uname().nodename.split(".")[0]]
        self.assertEqual(stale["quality"], "stale")
        self.assertIn("exceeds ttl", stale["reason"])
        self.assertEqual(model.epoch(stale["observed_at"]), NOW - 10 * 86400)

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
        # reviews has a real collector now; an enrolled-but-missing source
        # must read as absent-unavailable, never as implemented-empty
        self.assertEqual(cov["reviews:rev:" + local]["quality"], "unavailable")
        self.assertIn("absent", cov["reviews:rev:" + local]["reason"])
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


class ComposerInspectors(unittest.TestCase):
    """Composer / reviews / memory-lock collectors, the workers query route,
    the dynamic instance roster, conditional-ledger presentation, and the
    negative falsifiers: material unknown, absent, stale, config fail, forged
    PASS, cursor/query tamper, private fields, one sealed frame for composer
    and operator alike."""

    def setUp(self):
        self.fix = fixture(self.__class__.__name__)
        self.h1, self.h2 = hermetic(self.fix)

    def tearDown(self):
        shutil.rmtree(self.fix, ignore_errors=True)

    def _frame(self):
        return adapters.capture(now=NOW, root=self.fix / "store")

    def test_repeated_dedupe_key_is_one_observation_not_a_second_vote(self):
        d = self.fix / "composer"
        d.mkdir(parents=True)
        obs = [{"at": "2026-10-27T00:00:00Z", "dedupe_key": "k1", "detector_id": "sig-x",
                "detector_version": "det-v3", "fingerprint": "f1", "generation": "g1",
                "lineage_id": "l1", "severity": "medium", "source_kind": "board",
                "span": "bytes@1", "status": "unverified-symptom", "leak": "body"},
               {"at": "2026-10-27T01:00:00Z", "dedupe_key": "k1", "detector_id": "sig-x",
                "detector_version": "det-v3", "fingerprint": "f1", "generation": "g2",
                "lineage_id": "l1", "severity": "medium", "source_kind": "board",
                "span": "bytes@2", "status": "unverified-symptom", "leak": "body2"}]
        (d / "observations.jsonl").write_text(
            "\n".join(json.dumps(o) for o in obs) + "\n")
        with self.h1, self.h2:
            rows, state, detail = collectors.collect_composer(NOW)
        self.assertEqual(state, "ok")
        k1 = [r for r in rows if r.get("row_type") == "observation"]
        self.assertEqual(len(k1), 1)
        self.assertEqual(k1[0]["repeat_count"], 2)
        self.assertEqual(k1[0]["detector_version"], "det-v3")
        self.assertNotIn("leak", k1[0])  # closed OBS fields only

    def test_absent_outbox_is_material_unknown_not_empty_ok(self):
        with self.h1, self.h2:
            rows, state, detail = collectors.collect_composer(NOW)
        self.assertEqual(detail["outbox_state"], "absent")
        self.assertEqual([r for r in rows if r.get("id") == "composer-state:1"][0]
                         ["outbox_state"], "absent")

    def test_cursor_sources_are_fingerprinted_never_raw_paths(self):
        d = self.fix / "composer"
        d.mkdir(parents=True)
        (d / "cursors.json").write_text(json.dumps(
            {"/home/user/example/boards/secret-topic/board.jsonl": {
                "cursor": 42, "generation": "sz42:aa:bb", "lineage_id": "lx"}}))
        with self.h1, self.h2:
            rows, state, _ = collectors.collect_composer(NOW)
        cur = [r for r in rows if r.get("row_type") == "cursor"][0]
        self.assertEqual(cur["cursor"], 42)
        self.assertEqual(cur["source_label"], "board.jsonl")
        self.assertNotIn("/home/user", json.dumps(cur))

    def test_stale_composer_observation_demotes_coverage(self):
        d = self.fix / "composer"
        d.mkdir(parents=True)
        old = model.iso(NOW - 10 * 86400)
        (d / "observations.jsonl").write_text(json.dumps(
            {"at": old, "dedupe_key": "k", "detector_id": "d", "detector_version": "v",
             "fingerprint": "f", "generation": "g", "lineage_id": "l",
             "severity": "low", "source_kind": "board", "span": "s",
             "status": "unverified-symptom"}) + "\n")
        with self.h1, self.h2:
            frame = self._frame()
        cov = {c["source"]: c for c in frame["coverage"]}
        self.assertEqual(cov["composer:" + self.local]["quality"], "stale")

    def test_missing_composer_source_is_unavailable_not_healthy(self):
        with self.h1, self.h2:
            frame = self._frame()
        cov = {c["source"]: c for c in frame["coverage"]}
        self.assertEqual(cov["composer:" + self.local]["quality"], "unavailable")
        self.assertIn("absent", cov["composer:" + self.local]["reason"])

    def test_reviews_chain_level_rows_and_no_repeat_votes(self):
        d = self.fix / "reviews"
        d.mkdir(parents=True)
        build_chain(d, "fix-review", [
            {"kind": "review_opened", "review_id": "fix-review", "tier": "full",
             "at": "2026-10-28T00:00:00Z"},
            {"kind": "editor_verdict", "snapshot_digest": "d" * 64,
             "editor": {"name": "ed1"}, "overall": "COMPLETE",
             "signed_payload": {}, "verification_head": GENESIS,
             "at": "2026-10-28T00:01:00Z"},
            {"kind": "editor_verdict", "snapshot_digest": "d" * 64,
             "editor": {"name": "ed2"}, "overall": "COMPLETE",
             "signed_payload": {}, "verification_head": GENESIS,
             "at": "2026-10-28T00:02:00Z"},
            {"kind": "resolution", "outcome": "COMPLETE",
             "at": "2026-10-28T00:03:00Z"}])
        with self.h1, self.h2:
            rows, state, _ = collectors.collect_reviews(NOW)
        self.assertEqual(state, "ok")
        self.assertEqual(len(rows), 1)  # one chain = one row, never a verdict tally
        self.assertEqual(rows[0]["kinds"]["editor_verdict"], 2)
        self.assertEqual(rows[0]["editor_bound"], {"name": "ed2"}["name"])
        self.assertEqual(rows[0]["resolution"], "COMPLETE")
        self.assertEqual(rows[0]["review_state"], "verdict-claimed")

    def test_review_claim_never_reads_editor_verified_without_receipt(self):
        d = self.fix / "reviews"
        d.mkdir(parents=True)
        good_chain(d, "fix-complete")
        with self.h1, self.h2:
            rows, _, _ = collectors.collect_reviews(NOW)
            frame = self._frame()
        self.assertNotEqual(rows[0]["review_state"], "editor-verified")
        self.assertTrue(all(r.get("verification") != "editor-verified"
                            for r in frame["datasets"]["reviews"]))

    def test_memory_lock_absent_is_explicit_unknown_finding(self):
        with self.h1, self.h2:
            frame = self._frame()
        cov = {c["source"]: c for c in frame["coverage"]}
        self.assertEqual(cov["memory-lock:" + self.local]["quality"], "unavailable")
        self.assertIn("finding:memory-lock:", json.dumps(frame["datasets"]["findings"]))

    def test_memory_lock_lists_sealed_modules_with_digests(self):
        kd = self.fix / "memlock"
        kd.mkdir(parents=True)
        (kd / "composer.py").write_text("# sealed fixture\n")
        (kd / "session_review.py").write_text("# sealed fixture\n")
        with self.h1, self.h2:
            rows, state, _ = collectors.collect_memory_lock(NOW)
            frame = self._frame()
        self.assertEqual(state, "ok")
        self.assertEqual(len(rows), 2)
        self.assertTrue(all(r["sha256_16"] for r in rows))
        self.assertIn("memory-lock", [e.get("subsystem")
                                      for e in frame["datasets"]["events"]])

    local = property(lambda self: os.uname().nodename.split(".")[0])

    def test_failed_work_keeps_unreviewed_explicit_with_refs(self):
        d = self.fix / "dreamer"
        d.mkdir(parents=True)
        (d / "audit-ledger.jsonl").write_text(json.dumps(
            {"id": "q1", "verdict": "PASS", "repair_ref": "rr-1",
             "audited_at": model.iso(NOW - 60), "review_id": "no-such-review"}) + "\n")
        import introspection.test_inspection_repair as t
        with self.h1, self.h2:
            frame = self._frame()
        reps = frame["datasets"]["repairs"]
        self.assertTrue(reps)
        self.assertEqual(reps[0]["verification"], "unverified")
        self.assertIn("review_id", reps[0])
        self.assertTrue(all(r.get("verification_reason")
                            for r in reps if r["verification"] != "editor-verified"))

    def test_worker_query_family_routes_and_filters(self):
        workers = [{"id": "worker:one", "origin": "manual-owner-worker",
                    "status": "claimed", "updated_at": model.iso(NOW)},
                   {"id": "worker:two", "origin": "manual-owner-worker"}]
        cov = [model.coverage("workers-registry:h", "workers", "workers",
                              observed_at=model.iso(NOW),
                              received_at=model.iso(NOW), ttl=60)]
        frame = model.frame(model.digest({"x": 1}), model.iso(NOW), cov,
                            {"workers": workers,
                             "dreams": [], "attempts": [], "services": [], "schedules": [],
                             "plans": [], "events": [], "heartbeat": [], "findings": [],
                             "repairs": [], "composer": [], "reviews": [], "summary": []})
        res, code = model.query(frame, ("worker", "ps"), {}, NOW)
        self.assertEqual(len(res["items"]), 2)
        res, code = model.query(frame, ("worker", "ls"), {"id": "worker:two"}, NOW)
        self.assertEqual([r["id"] for r in res["items"]], ["worker:two"])

    def test_cursor_tamper_across_snapshots_rejected(self):
        rows = [{"id": "worker:a"}, {"id": "worker:b"}, {"id": "worker:c"}]
        cov = [model.coverage("workers-registry:h", "workers", "workers",
                              observed_at=model.iso(NOW),
                              received_at=model.iso(NOW), ttl=60)]
        empty = {"dreams": [], "attempts": [], "services": [], "schedules": [],
                 "plans": [], "events": [], "heartbeat": [], "findings": [],
                 "repairs": [], "composer": [], "reviews": [], "summary": []}
        f1 = model.frame("a" * 64, model.iso(NOW), cov,
                         dict(empty, workers=rows))
        f2 = model.frame("b" * 64, model.iso(NOW), cov, dict(empty, workers=rows))
        res, _ = model.query(f1, ("worker", "ls"), {"limit": 1}, NOW)
        tok = res["next_cursor"]
        self.assertTrue(tok)
        with self.assertRaises(ValueError):
            model.query(f2, ("worker", "ls"), {"limit": 1, "cursor": tok}, NOW)
        bad = tok[:-2] + ("AA" if tok[-2:] != "AA" else "BB")
        with self.assertRaises(ValueError):
            model.query(f1, ("worker", "ls"), {"limit": 1, "cursor": bad}, NOW)

    def test_private_fields_refused_in_new_datasets(self):
        bad = model.frame("a" * 64, model.iso(NOW), [],
                          {"composer": [{"id": "x", "prompt": "secret body"}],
                           "summary": []})
        with self.assertRaises(ValueError):
            model.validate_frame(bad)
        bad2 = model.frame("a" * 64, model.iso(NOW), [],
                           {"reviews": [{"id": "x", "access_token": "sk-abcdefghijklmnop"}],
                            "summary": []})
        with self.assertRaises(ValueError):
            model.validate_frame(bad2)

    def test_same_sealed_frame_for_composer_and_operator(self):
        datasets = {k: [] for k in ("dreams", "attempts", "services", "schedules",
                                    "plans", "events", "heartbeat", "findings",
                                    "repairs", "workers", "composer", "reviews",
                                    "summary")}
        frame = model.frame("a" * 64, model.iso(NOW), [], datasets)
        r1, c1 = model.query(frame, ("worker", "ls"), {}, NOW)
        r2, c2 = model.query(frame, ("worker", "ls"), {}, NOW + 500)
        self.assertEqual(r1["snapshot_id"], r2["snapshot_id"])
        self.assertEqual(json.dumps(r1["items"]), json.dumps(r2["items"]))
        self.assertEqual(c1, c2)

    def test_dynamic_executor_instances_are_discovered_not_invented(self):
        calls = []
        def fake_systemctl(argv, timeout):
            calls.append(argv)
            if argv[0] == "show":
                out = ""
                for u in argv[1:]:
                    if u.endswith(".service") and "@" in u \
                            and not u.endswith("@.service"):
                        out += ("Id=%s\nActiveState=active\nSubState=running\n"
                                "LoadState=loaded\nMainPID=0\n\n" % u)
                return out, "ok"
            if any("*" in a for a in argv):
                return ("ygg-swarm-executor@1.service loaded active running\n"
                        "ygg-swarm-executor@7.service loaded active running\n"), "ok"
            return ("", "ok")
        with mock.patch.object(collectors, "_systemctl", fake_systemctl):
            rows, state, detail = collectors.collect_services(
                ["ygg-swarm-executor@.service"], NOW)
        self.assertEqual(state, "ok")
        self.assertEqual(detail["dynamic_instances"],
                         ["ygg-swarm-executor@1.service", "ygg-swarm-executor@7.service"])
        self.assertIn("ygg-swarm-executor@.service", detail["expected_not_found"])
        units = {r["unit"] for r in rows}
        self.assertIn("ygg-swarm-executor@1.service", units)

    def test_conditional_ledger_presented_never_executed(self):
        import types
        executed = []
        stub = types.ModuleType("swarm_ledger")
        stub.journal_records = lambda state, area="config": (
            [{"change_id": "c1", "revision": 2, "principal": "orchestrator",
              "authority_ref": "owner"}] if area == "config" else
            ([{"plan_id": "p1", "plan": "reroute quota-blocked queue",
               "predicate": "quota reset window observed", "deadline": model.iso(NOW + 3600),
               "priority": "high", "source": "orchestrator-plan-2026-10-27",
               "authority_ref": "owner", "timing": "on-quota-reset"}]
             if area == "conditional" else []))
        stub.verify_ancestry = lambda state, area="config": (True, None)
        with mock.patch.dict(sys.modules, {"swarm_ledger": stub}):
            rows, state, detail = collectors.collect_commitments(str(self.fix), NOW)
        cond = [r for r in rows if r.get("kind") == "conditional-plan"]
        self.assertEqual(len(cond), 1)
        self.assertEqual(cond[0]["predicate"], "quota reset window observed")
        self.assertEqual(cond[0]["priority"], "high")
        self.assertEqual(cond[0]["authority"], "owner")
        self.assertEqual(cond[0]["execution"],
                         "presentation-only (never executed by the read adapter)")
        # prose intent is not misread as a hardcoded schedule
        self.assertFalse(cond[0]["programmatic"])
        self.assertEqual(executed, [])  # nothing ran

    def test_prune_never_deletes_review_cited_snapshots(self):
        cited = "d" * 64
        d = self.fix / "reviews"
        d.mkdir(parents=True)
        build_chain(d, "citing", [
            {"kind": "review_opened", "review_id": "citing", "tier": "full",
             "at": "2026-10-28T00:00:00Z"},
            {"kind": "snapshot_staged", "subject_digest": cited, "round": 1,
             "completion_claim": {"text": "x"}, "at": "2026-10-28T00:01:00Z"},
            {"kind": "resolution", "outcome": "COMPLETE",
             "at": "2026-10-28T00:02:00Z"}])
        sd = store.snapshots_dir(self.fix / "store")
        sd.mkdir(parents=True, exist_ok=True)
        keep = sd / (cited + ".json")
        keep.write_text("{}")
        old = NOW - 90 * 86400
        os.utime(keep, (old, old))
        gone = sd / ("e" * 64 + ".json")
        gone.write_text("{}")
        os.utime(gone, (old, old))
        with self.h1, self.h2:
            removed = store.prune(root=self.fix / "store", now=NOW,
                                  retention={"raw_days": 31})
        self.assertFalse(gone.exists())
        self.assertTrue(keep.exists())
        self.assertGreaterEqual(removed["protected_by_review"], 1)


_EMPTY = {k: [] for k in ("dreams", "attempts", "services", "schedules", "plans",
                          "events", "heartbeat", "findings", "repairs", "workers",
                          "composer", "reviews", "summary")}


class ObservationFixFalsifiers(unittest.TestCase):
    """The seven Gemini-accepted negatives, the parent review-association
    binding, and the pinned-frame/physical-attempt dream-open contract, as
    executable repairs (2026-10-28)."""

    def setUp(self):
        self.fix = fixture(self.__class__.__name__)
        self.h1, self.h2 = hermetic(self.fix)
        self.local = os.uname().nodename.split(".")[0]
        self.root = self.fix / "store"

    def tearDown(self):
        shutil.rmtree(self.fix, ignore_errors=True)

    def _cfg(self, sub):
        return {"mode": "active", "config": {"fleet": {}, "introspection": sub},
                "faults": []}

    # --- F1: standing coverage carries the SOURCE clock, never the reader's
    def test_standing_maintenance_coverage_uses_supervisor_pass_time(self):
        sup = self.fix / "swarm" / "supervisor.json"
        sup.parent.mkdir(parents=True, exist_ok=True)
        old = model.iso(NOW - 2 * 86400)
        sup.write_text(json.dumps({"at": old,
                                   "executor": {"unit": "u.service", "active": True,
                                                "action": "n"}}))
        with self.h1, self.h2:
            frame = adapters.capture(now=NOW, root=self.root)
        cov = {c["source"]: c for c in frame["coverage"]}
        m = cov["maintenance:" + self.local]
        self.assertEqual(model.epoch(m["observed_at"]), model.epoch(old))
        self.assertEqual(m["quality"], "stale")
        self.assertEqual(model.epoch(m["received_at"]), NOW)

    # --- F2: configured capture reads nothing unenrolled
    def test_configured_capture_reads_neither_workers_nor_pressure_unenrolled(self):
        sub = {"version": 1, "enabled": True, "sources": {
            "hb-off": {"kind": "heartbeat", "host": "local", "enabled": False,
                       "ttl_seconds": 60, "path": "/unused"}}}

        def boom(*a, **k):
            raise AssertionError("unenrolled collector invoked")

        with self.h1, self.h2:
            with mock.patch("swarmharness.swarmconfig.load", return_value=self._cfg(sub)), \
                 mock.patch.object(adapters.collectors, "collect_workers", boom), \
                 mock.patch.object(adapters.collectors, "collect_pressure", boom):
                frame = adapters.capture(now=NOW, root=self.root)
        sources = [c["source"] for c in frame["coverage"]]
        self.assertNotIn("workers-registry:" + self.local, sources)
        self.assertNotIn("pressure:" + self.local, sources)
        self.assertEqual(frame["datasets"]["workers"], [])
        self.assertEqual(frame["datasets"]["plans"], [])

    def test_configured_workers_source_is_enrollment_scoped(self):
        reg = self.fix / "rows"
        reg.mkdir(parents=True)
        (reg / "w1.json").write_text(json.dumps({"id": "w1", "status": "claimed"}))
        os.utime(reg / "w1.json", (NOW - 10, NOW - 10))
        sub = {"version": 1, "enabled": True, "sources": {
            "reg": {"kind": "workers", "host": "local", "enabled": True,
                    "ttl_seconds": 60, "path": str(reg)}}}
        with self.h1, self.h2:
            with mock.patch("swarmharness.swarmconfig.load", return_value=self._cfg(sub)):
                frame = adapters.capture(now=NOW, root=self.root)
        cov = {c["source"]: c for c in frame["coverage"]}
        row = cov["workers:reg:" + self.local]
        self.assertEqual(row["quality"], "verified")
        self.assertEqual(model.epoch(row["observed_at"]), NOW - 10)
        self.assertEqual(len(frame["datasets"]["workers"]), 1)

    # --- F2b/--host all: remote enrolled host is expected-unknown coverage,
    # never folded into local-only coverage
    def test_configured_remote_host_is_unknown_coverage_not_local(self):
        sub = {"version": 1, "enabled": True, "sources": {
            "svc": {"kind": "services", "host": "oc", "enabled": True,
                    "ttl_seconds": 60, "units": ["x.service"]}}}
        with self.h1, self.h2:
            with mock.patch("swarmharness.swarmconfig.load", return_value=self._cfg(sub)):
                frame = adapters.capture(now=NOW, root=self.root)
        row = {c["source"]: c for c in frame["coverage"]}["services:svc:" + self.local]
        self.assertEqual(row["host"], "oc")
        self.assertEqual(row["quality"], "unknown")
        self.assertIn("not silently widened", row["reason"])
        res, code = model.query(frame, ("service", "ps"), {"host": "all"}, NOW)
        self.assertIn("oc", {c["host"] for c in res["coverage"]})
        self.assertEqual(code, 3)  # an all-hosts scope with unknown hosts is not complete
        res, _ = model.query(frame, ("service", "ps"), {"host": "oc"}, NOW)
        self.assertTrue(any(c["host"] == "oc" for c in res["coverage"]))

    # --- F3: refresh forces an exact new frame; the pin cannot be bypassed
    def test_refresh_forces_a_new_frame_despite_fresh_cache(self):
        with self.h1, self.h2:
            first = adapters.capture(now=NOW, root=self.root)
            real = adapters.capture
            seen = []
            def spy(now=None, root=None):
                seen.append(1)
                return real(now=now, root=root)
            def serve_json(args):
                buf = io.StringIO()
                with mock.patch.object(adapters, "capture", spy), \
                     contextlib.redirect_stdout(buf):
                    adapters.serve(("swarm", "status"), dict(args, json=True),
                                   root=self.root)
                return json.loads(buf.getvalue())
            refreshed = serve_json({"refresh": True})
            self.assertTrue(seen)  # capture ran despite the fresh cached frame
            self.assertNotEqual(refreshed["snapshot_id"], first["snapshot_id"])
            seen.clear()
            serve_json({})
            self.assertFalse(seen)  # fresh cache reused without refresh
            seen.clear()
            pinned = serve_json({"refresh": True, "snapshot": first["snapshot_id"]})
            self.assertFalse(seen)  # an explicit snapshot pin cannot be bypassed
            self.assertEqual(pinned["snapshot_id"], first["snapshot_id"])

    # --- F4: --all --since keeps work whose historical attempt is in window
    def test_dream_all_since_keeps_work_whose_attempt_is_in_window(self):
        old_parent = {"id": "work-old", "state": "completed",
                      "created_at": model.iso(NOW - 2 * 86400),
                      "updated_at": model.iso(NOW - 2 * 86400),
                      "attempt_ids": ["attempt-recent"]}
        recent = {"id": "attempt-recent", "work_id": "work-old", "state": "failed",
                  "finished_at": NOW - 600}
        stale_parent = {"id": "work-stale", "state": "failed",
                        "created_at": model.iso(NOW - 2 * 86400),
                        "updated_at": model.iso(NOW - 2 * 86400),
                        "attempt_ids": ["attempt-ancient"]}
        ancient = {"id": "attempt-ancient", "work_id": "work-stale",
                   "state": "failed", "finished_at": NOW - 2 * 86400}
        cov = [model.coverage("queue:h", "queue", "queue", observed_at=model.iso(NOW),
                              received_at=model.iso(NOW), ttl=60)]
        frame = model.frame("a" * 64, model.iso(NOW), cov,
                            dict(_EMPTY, dreams=[old_parent, stale_parent],
                                 attempts=[recent, ancient]))
        res, _ = model.query(frame, ("dream", "ls"), {"all": True, "since": "1h"}, NOW)
        self.assertEqual([r["id"] for r in res["items"]], ["work-old"])
        res, _ = model.query(frame, ("dream", "ls"), {"all": True, "since": "3d"}, NOW)
        self.assertEqual(sorted(r["id"] for r in res["items"]),
                         ["work-old", "work-stale"])

    # --- F5: partition retention is evaluated in UTC
    def test_partition_end_is_utc_not_local(self):
        old_tz = os.environ.get("TZ")
        try:
            os.environ["TZ"] = "Asia/Kolkata"
            time.tzset()
            self.assertEqual(store._partition_end("2026-10"),
                             calendar.timegm((2026, 11, 1, 0, 0, 0, 0, 0, 0)))
            self.assertEqual(store._partition_end("2026-12"),
                             calendar.timegm((2027, 1, 1, 0, 0, 0, 0, 0, 0)))
        finally:
            if old_tz is None:
                os.environ.pop("TZ", None)
            else:
                os.environ["TZ"] = old_tz
            time.tzset()

    # --- F6: dedupe read + append are one critical section
    def test_event_dedupe_read_happens_inside_the_lock(self):
        order = []
        month = store.events_partition(NOW)
        real_read = Path.read_text
        real_flock = store.fcntl.flock
        def spy_read(path_self, *a, **k):
            if path_self.name == month:
                order.append("read")
            return real_read(path_self, *a, **k)
        def spy_flock(fd, op):
            if op == store.fcntl.LOCK_EX:
                order.append("lock")
            return real_flock(fd, op)
        with mock.patch.object(Path, "read_text", spy_read), \
             mock.patch.object(store.fcntl, "flock", spy_flock):
            store.append_events([{"id": "ev-race", "summary": "s"}],
                                root=self.root, now=NOW)
        self.assertEqual(order[:2], ["lock", "read"])

    def test_two_writers_same_id_append_once(self):
        results = []
        barrier = threading.Barrier(2)
        def writer():
            barrier.wait()
            results.append(store.append_events(
                [{"id": "ev-dup", "summary": "x"}], root=self.root, now=NOW))
        threads = [threading.Thread(target=writer) for _ in range(2)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertEqual(sum(r["appended"] for r in results), 1)
        self.assertEqual(len(store.read_events(root=self.root)), 1)

    # --- F7: ONE clock-skew policy across capture, collectors and query
    def test_one_clock_skew_policy_across_capture_collect_query(self):
        future = NOW + 100  # inside the old loose gate, outside the strict one
        row = model.coverage("s", "h", "queue", observed_at=model.iso(future),
                             received_at=model.iso(NOW), ttl=60)
        self.assertEqual(model.assess(row, NOW)["quality"], "conflicting")
        self.assertEqual(collectors.plane_fresh({"observed_at": future}, NOW, 60),
                         "unknown")
        cov = adapters._coverage("s", "queue", "ok", NOW, 60,
                                 source_observed_at=future)
        self.assertEqual(cov["quality"], "conflicting")
        within = NOW + 1  # inside the shared bound: certifiable everywhere
        self.assertEqual(adapters._coverage("s", "queue", "ok", NOW, 60,
                                            source_observed_at=within)["quality"],
                         "verified")
        self.assertEqual(collectors.plane_fresh({"observed_at": within}, NOW, 60),
                         "current")
        ok = model.coverage("s", "h", "queue", observed_at=model.iso(within),
                            received_at=model.iso(NOW), ttl=60)
        self.assertEqual(model.assess(ok, NOW)["quality"], "verified")

    # --- trusted review association: same work or unverified
    def _bound_chain(self, rid, session, subject):
        from swarmharness import session_review as sr
        key_path = self.fix / "control.key"
        key_path.write_bytes(b"fixture-control-key\n")
        opened = {"kind": "review_opened", "review_id": rid, "session": session,
                  "worker": "fixture-worker", "tier": "full",
                  "at": "2026-10-28T00:00:00Z"}
        snap = {"kind": "snapshot_staged", "subject_digest": subject, "round": 1,
                "completion_claim": {"text": "fixture"},
                "at": "2026-10-28T00:01:00Z"}
        editor = {"name": "fixture-editor"}
        payload = {"overall": "COMPLETE", "findings": []}
        # Chain the opened/snapshot rows FIRST and sign over those exact rows:
        # the verifier (like the kernel's ingest_verdict) presents the chain
        # rows with their seq/prev_hash/record_hash bookkeeping, so the
        # receipt binds the same bytes a reader will replay.
        chained = []
        prev = GENESIS

        def append(body):
            nonlocal prev
            row = dict(body, seq=len(chained), prev_hash=prev)
            row["record_hash"] = sr._sha(sr._canonical(row))
            chained.append(row)
            prev = row["record_hash"]

        append(opened)
        append(snap)
        head = prev  # the chain head the verdict verifies against
        receipt = sr.editor_receipt(rid, chained[0], chained[1], editor,
                                    payload, key=key_path.read_bytes().strip(),
                                    review_head=head)
        verdict = {"kind": "editor_verdict", "snapshot_digest": subject,
                   "editor": dict(editor, invocation_receipt=receipt),
                   "overall": "COMPLETE", "signed_payload": payload,
                   "verification_head": head, "approval_scope": "full",
                   "findings": [], "at": "2026-10-28T00:02:00Z"}
        append(verdict)
        append({"kind": "resolution", "outcome": "COMPLETE",
                "at": "2026-10-28T00:03:00Z"})
        path = self.fix / "reviews" / (rid + ".jsonl")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(json.dumps(r, sort_keys=True)
                                  for r in chained) + "\n")
        return key_path

    def test_bound_valid_review_makes_repair_editor_verified(self):
        subject = "a" * 64
        key_path = self._bound_chain("rev-bound", "sess-fix-1", subject)
        state, why = adapters._receipt_state(
            "rev-bound", self.fix / "reviews", key_path=key_path,
            claim={"session_id": "sess-fix-1", "subject_digest": subject})
        self.assertEqual(state, "editor-verified", why)

    def test_copied_valid_review_for_different_work_stays_unverified(self):
        subject = "b" * 64
        key_path = self._bound_chain("rev-copy", "sess-other-work", subject)
        state, why = adapters._receipt_state(
            "rev-copy", self.fix / "reviews", key_path=key_path,
            claim={"session_id": "sess-this-work", "subject_digest": subject})
        self.assertEqual(state, "unverified")
        self.assertIn("not the audit row's session", why)
        state, why = adapters._receipt_state(
            "rev-copy", self.fix / "reviews", key_path=key_path,
            claim={"session_id": "sess-other-work", "subject_digest": "c" * 64})
        self.assertEqual(state, "unverified")
        self.assertIn("subject", why)
        state, why = adapters._receipt_state(
            "rev-copy", self.fix / "reviews", key_path=key_path,
            claim={"session_id": "sess-other-work"})
        self.assertEqual(state, "unverified")
        self.assertIn("subject digest", why)

    # --- F2c: a served enrolled source records its ENROLLED host, never the
    # coverage default — host filtering must retrieve what enrollment bound
    def test_enrolled_explicit_host_is_recorded_on_served_source(self):
        ledger = self.fix / "tasks.md"
        ledger.write_text("## T-1 fixture\nstatus: open\n")
        os.utime(ledger, (NOW - 10, NOW - 10))
        sub = {"version": 1, "enabled": True, "sources": {
            "ledger": {"kind": "task_ledger", "host": self.local,
                       "enabled": True, "ttl_seconds": 60,
                       "path": str(ledger)}}}
        with self.h1, self.h2:
            with mock.patch("swarmharness.swarmconfig.load", return_value=self._cfg(sub)):
                frame = adapters.capture(now=NOW, root=self.root)
        row = {c["source"]: c for c in frame["coverage"]}[
            "task_ledger:ledger:" + self.local]
        self.assertEqual(row["host"], self.local)
        self.assertEqual(row["quality"], "verified")

    # --- binding: staging is not approving. A chain that staged subject A
    # then B with a verdict for B must never verify a claim for A.
    def _two_snapshot_chain(self, rid, session, subjects, approved_index):
        from swarmharness import session_review as sr
        key_path = self.fix / ("control-" + rid + ".key")
        key_path.write_bytes(b"fixture-control-key\n")
        chained = []
        prev = GENESIS

        def append(body):
            nonlocal prev
            row = dict(body, seq=len(chained), prev_hash=prev)
            row["record_hash"] = sr._sha(sr._canonical(row))
            chained.append(row)
            prev = row["record_hash"]

        append({"kind": "review_opened", "review_id": rid, "session": session,
                "worker": "fixture-worker", "tier": "full",
                "at": "2026-10-28T00:00:00Z"})
        for i, subject in enumerate(subjects):
            append({"kind": "snapshot_staged", "subject_digest": subject,
                    "round": i + 1, "completion_claim": {"text": "fixture"},
                    "at": "2026-10-28T00:0%d:00Z" % (i + 1)})
        editor = {"name": "fixture-editor"}
        payload = {"overall": "COMPLETE", "findings": []}
        approved = chained[approved_index]
        receipt = sr.editor_receipt(rid, chained[0], approved, editor, payload,
                                    key=key_path.read_bytes().strip(),
                                    review_head=prev)
        append({"kind": "editor_verdict", "snapshot_digest": approved["subject_digest"],
                "editor": dict(editor, invocation_receipt=receipt),
                "overall": "COMPLETE", "signed_payload": payload,
                "verification_head": prev, "approval_scope": "full",
                "findings": [], "at": "2026-10-28T00:09:00Z"})
        append({"kind": "resolution", "outcome": "COMPLETE",
                "at": "2026-10-28T00:10:00Z"})
        path = self.fix / "reviews" / (rid + ".jsonl")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(json.dumps(r, sort_keys=True)
                                  for r in chained) + "\n")
        return key_path

    def test_binding_requires_the_verdicts_approved_subject(self):
        subjects = ["a" * 64, "b" * 64]
        key_path = self._two_snapshot_chain("rev-approve-b", "sess-fix-1",
                                            subjects, approved_index=2)
        state, why = adapters._receipt_state(
            "rev-approve-b", self.fix / "reviews", key_path=key_path,
            claim={"session_id": "sess-fix-1", "subject_digest": subjects[0]})
        self.assertEqual(state, "unverified", "the staged-but-unapproved A verified")
        self.assertIn("approved subject digest", why)
        state, why = adapters._receipt_state(
            "rev-approve-b", self.fix / "reviews", key_path=key_path,
            claim={"session_id": "sess-fix-1", "subject_digest": subjects[1]})
        self.assertEqual(state, "editor-verified", why)

    def test_repairs_never_verify_from_unrelated_complete_review(self):
        key_path = self._bound_chain("rev-unrel", "sess-x", "d" * 64)
        d = self.fix / "dreamer"
        d.mkdir(parents=True)
        (d / "audit-ledger.jsonl").write_text(json.dumps(
            {"id": "t1", "repair_ref": "r1", "verdict": "PASS",
             "audited_at": NOW, "review_id": "rev-unrel",
             "session_id": "sess-y", "subject_digest": "e" * 64}) + "\n")
        with self.h1, self.h2:
            frame = adapters.capture(now=NOW, root=self.root)
        rep = frame["datasets"]["repairs"][0]
        self.assertEqual(rep["verification"], "unverified")
        self.assertIn("session", rep["verification_reason"])

    # --- dream open: exact pinned frame, exact physical attempt, no fake jump
    def test_dream_open_uses_the_pinned_frame_and_physical_attempt(self):
        cov = [model.coverage("queue:h", "queue", "queue", observed_at=model.iso(NOW),
                              received_at=model.iso(NOW), ttl=60)]
        work = {"id": "work:queue:t9", "queue_id": "t9", "state": "failed",
                "current_attempt_id": "attempt:old",
                "attempt_ids": ["attempt:old", "attempt:new"]}
        a_old = {"id": "attempt:old", "work_id": "work:queue:t9", "state": "failed",
                 "evidence_state": "verified", "session_id": "sess-old"}
        a_new = {"id": "attempt:new", "work_id": "work:queue:t9",
                 "state": "completed", "evidence_state": "verified",
                 "session_id": "sess-new"}
        old_frame = model.frame("a" * 64, model.iso(NOW), cov,
                                dict(_EMPTY, dreams=[work], attempts=[a_old]))
        new_frame = model.frame("b" * 64, model.iso(NOW), cov,
                                dict(_EMPTY, dreams=[dict(work, current_attempt_id="attempt:new")],
                                     attempts=[a_new]))
        store.save_snapshot(old_frame, root=self.root)
        store.save_snapshot(new_frame, root=self.root)
        def no_latest(*a, **k):
            raise AssertionError("latest_snapshot consulted despite pinned frame")
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            with mock.patch.object(store, "latest_snapshot", no_latest):
                result, code = adapters.dream_open(
                    "work:queue:t9", {"attempt": "attempt:old"},
                    frame=old_frame, root=self.root)
        self.assertEqual(code, 3)  # a known native session still cannot fake a jump
        self.assertEqual(result["session_id"], "sess-old")
        with contextlib.redirect_stdout(out):
            result, _ = adapters.dream_open("work:queue:t9", {},
                                            frame=old_frame, root=self.root)
        self.assertEqual(result["session_id"], "sess-old")  # current attempt IN the pin
        with contextlib.redirect_stdout(out):
            result, code = adapters.dream_open(
                "work:queue:t9", {"attempt": "attempt:new"},
                frame=old_frame, root=self.root)
        self.assertEqual(code, 3)
        self.assertIn("not in the selected snapshot", result["error"])
