"""Negative controls for identity, immutable snapshots and partial evidence."""
from copy import deepcopy
import unittest

from . import model as m

NOW = 1_900_000_000


class InspectionModel(unittest.TestCase):
    def capture(self, rows=None, quality="verified", age=0):
        c = m.coverage("queue-example", "worker-a", "queue", observed_at=m.iso(NOW - age),
                       received_at=m.iso(NOW), ttl=60, quality=quality, revision="receipt-1")
        return m.frame("a" * 64, m.iso(NOW), [c], {"dreams": rows or [], "attempts": []})

    def test_zero_stale_unknown_and_future_sources_are_different(self):
        for quality, age, expected in (("verified", 0, 0), ("unavailable", 0, 3),
                                      ("verified", 100, 3), ("verified", -100, 3)):
            with self.subTest(quality=quality, age=age):
                snapshot = self.capture(quality=quality, age=age)
                result, code = m.query(snapshot, ("dream", "ls"), {}, NOW)
                self.assertEqual(code, expected)
                self.assertEqual(result["items"], [])
                self.assertEqual(snapshot["coverage"][0]["quality"], quality)

    def test_snapshot_digest_and_metadata_enrollment_are_enforced(self):
        snapshot = self.capture()
        snapshot["datasets"]["dreams"].append({"id": "unexpected"})
        with self.assertRaisesRegex(ValueError, "digest"):
            m.validate_frame(snapshot)
        snapshot = self.capture([{"id": "work-1", "prompt": "private source body"}])
        with self.assertRaisesRegex(ValueError, "metadata"):
            m.validate_frame(snapshot)
        secret = "Bearer invented-fixture-credential"
        self.assertNotIn(secret, m.text(secret))
        snapshot = self.capture([{"id": "work-1", "title": secret}])
        with self.assertRaisesRegex(ValueError, "credential"):
            m.validate_frame(snapshot)

    def test_history_is_not_invented_from_retry_counter(self):
        tasks = [{"id": "task-1", "attempts": 6, "status": "failed", "last_error": "timeout"}]
        works, attempts = m.queue_records(tasks, [], {}, "queue-example", "worker-a", NOW)
        self.assertEqual(len(attempts), 1)
        self.assertEqual(works[0]["history_quality"], "unknown")
        self.assertEqual(attempts[0]["verification"], "unknown")
        self.assertEqual(attempts[0]["evidence_state"], "unavailable")

    def test_reused_ordinals_cannot_credit_old_attempt_with_new_transcript(self):
        metrics = [{"ev": "dispatch", "id": "task-1", "attempt": 1, "t": NOW - 40, "event_id": "first"},
                   {"ev": "done", "id": "task-1", "outcome": "budget-defer", "t": NOW - 30},
                   {"ev": "dispatch", "id": "task-1", "attempt": 1, "t": NOW - 10, "event_id": "second"}]
        tasks = [{"id": "task-1", "attempts": 1, "status": "running"}]
        metadata = {"task-1": {"quality": "verified", "ref": "evidence-1", "session_id": "session-new",
                              "harness": "fixture", "written_at": NOW}}
        _, attempts = m.queue_records(tasks, metrics, metadata, "queue-example", "worker-a", NOW)
        self.assertEqual(len(attempts), 2)
        self.assertNotEqual(attempts[0]["id"], attempts[1]["id"])
        self.assertEqual(attempts[0]["evidence_state"], "overwritten-or-unlinked")
        self.assertIsNone(attempts[0]["session_id"])
        self.assertEqual(attempts[1]["session_id"], "session-new")
        metadata["task-1"]["written_at"] = NOW - 20
        _, attempts = m.queue_records(tasks, metrics, metadata, "queue-example", "worker-a", NOW)
        self.assertIsNone(attempts[1]["session_id"])
        self.assertEqual(attempts[1]["evidence_state"], "stale-other-execution")

    def test_history_filter_matches_attempt_time_not_a_later_title_edit(self):
        old = {"id": "attempt-1", "work_id": "work-1", "state": "failed", "finished_at": NOW - 10 * 86400}
        row = {"id": "work-1", "state": "completed", "updated_at": NOW, "attempt_ids": [old["id"]]}
        snapshot = self.capture([row]);snapshot["datasets"]["attempts"] = [old]
        snapshot = m.frame(snapshot["config_hash"], snapshot["as_of"], snapshot["coverage"], snapshot["datasets"])
        result, _ = m.query(snapshot, ("dream", "ls"), {"all": True, "state": ["failed"], "since": "7d"}, NOW)
        self.assertEqual(result["items"], [])
        result, _ = m.query(snapshot, ("dream", "ls"), {"all": True, "state": ["failed"], "since": "14d"}, NOW)
        self.assertEqual(result["items"][0]["matching_attempt_ids"], ["attempt-1"])

    def test_pagination_is_bound_to_original_snapshot_and_query(self):
        first = self.capture([{"id": "work-1"}, {"id": "work-2"}])
        result, code = m.query(first, ("dream", "ls"), {"limit": 1}, NOW)
        self.assertEqual(code, 0)
        cursor = result["next_cursor"]
        page, _ = m.query(first, ("dream", "ls"), {"limit": 1, "cursor": cursor}, NOW)
        self.assertEqual(page["items"], [{"id": "work-2"}])
        newer = self.capture([{"id": "work-0"}, {"id": "work-1"}, {"id": "work-2"}])
        with self.assertRaisesRegex(ValueError, "another snapshot"):
            m.query(newer, ("dream", "ls"), {"limit": 1, "cursor": cursor}, NOW)
        with self.assertRaisesRegex(ValueError, "another snapshot"):
            m.query(first, ("dream", "ls"), {"limit": 1, "cursor": cursor, "host": "worker-a"}, NOW)

    def test_named_check_does_not_pass_missing_or_unknown_assertions(self):
        snapshot = self.capture()
        snapshot["datasets"]["findings"] = [{"scope": "dispatch", "verdict": "falsified"},
                                             {"scope": "services", "verdict": "unknown"}]
        snapshot = m.frame(snapshot["config_hash"], snapshot["as_of"], snapshot["coverage"], snapshot["datasets"])
        _, code = m.query(snapshot, ("swarm", "check"), {"scope": "all"}, NOW)
        self.assertEqual(code, 1)
        _, code = m.query(snapshot, ("swarm", "check"), {"scope": "services"}, NOW)
        self.assertEqual(code, 3)
        _, code = m.query(snapshot, ("swarm", "check"), {"scope": "funding"}, NOW)
        self.assertEqual(code, 3)


if __name__ == "__main__":
    unittest.main()
