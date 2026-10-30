"""Focused falsifiers for the standalone evidence-version store (CSR09).

Hermetic: SQLite stdlib only, disk-backed fixtures under
~/.yggterm/scratchpad (never /tmp), no network, no live corpus.
Every test names the spec falsifier it attacks."""

import hashlib
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import threading
import unittest
import uuid

import evidence_versions as ev


def _fixture_dir():
    root = os.path.expanduser("~/.yggterm/scratchpad/evidence-versions-tests")
    path = os.path.join(root, uuid.uuid4().hex)
    os.makedirs(path, exist_ok=True)
    return path


class EvidenceVersionsCase(unittest.TestCase):
    def setUp(self):
        self.dir = _fixture_dir()
        self.addCleanup(shutil.rmtree, self.dir, True)

    def source(self, **over):
        base = {"project": "/home/user/proj/widget-src",
                "source_kind": "log", "lineage_id": "42:7",
                "generation": "gen-a1"}
        base.update(over)
        return base

    def obs(self, **over):
        base = {"detector_id": "cmd-loop", "detector_version": "det-v3",
                "severity": "high", "status": "unverified-symptom",
                "span": "bytes@40", "fingerprint": "fp-normalized-1",
                "refs": [], "context": None}
        base.update(over)
        return base

    def _db_bytes(self):
        with open(self.db_file(), "rb") as fh:
            return fh.read()

    def db_file(self):
        return os.path.join(self.dir, ev.DB_NAME)

    def open_db(self):
        conn = sqlite3.connect(self.db_file())
        self.addCleanup(conn.close)
        return conn


class PreflightAndAbsentStore(EvidenceVersionsCase):
    def test_preflight_absent_never_initializes_or_writes(self):
        # Falsifier: preflight must not initialize or write an unknown store.
        self.assertEqual(ev.preflight(self.dir),
                         {"status": "absent", "schema_version": None})
        self.assertFalse(os.path.exists(self.db_file()))
        self.assertFalse(os.path.exists(self.dir + "-wal"))

    def test_preflight_ready_then_reads_schema(self):
        ev.record_page(self.dir, source=self.source(), start=0,
                       raw=b"alpha\n", context=None, observations=[])
        result = ev.preflight(self.dir)
        self.assertEqual(result["status"], "ready")
        self.assertEqual(result["schema_version"], ev.SCHEMA_VERSION)

    def test_future_schema_99_held_without_overwrite(self):
        # Falsifier: schema 99 / unknown future store holds, never rewrites.
        ev.record_page(self.dir, source=self.source(), start=0,
                       raw=b"seed\n", context=None, observations=[])
        conn = self.open_db()
        conn.execute("UPDATE meta SET value='99' "
                     "WHERE key='schema_version'")
        conn.commit()
        before = self._db_bytes()
        with self.assertRaises(ev.EvidenceHeld):
            ev.preflight(self.dir)
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=5,
                           raw=b"more\n", context=None, observations=[])
        self.assertEqual(self._db_bytes(), before)
        self.assertEqual(self.open_db().execute(
            "SELECT value FROM meta WHERE key='schema_version'"
        ).fetchone()[0], "99")

    def test_corrupt_bytes_held_without_overwrite(self):
        # Falsifier: corrupt store bytes hold non-destructively.
        os.makedirs(self.dir, exist_ok=True)
        with open(self.db_file(), "wb") as fh:
            fh.write(b"not a database at all" * 64)
        corrupt = self._db_bytes()
        with self.assertRaises(ev.EvidenceHeld):
            ev.preflight(self.dir)
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=0,
                           raw=b"x\n", context=None, observations=[])
        self.assertEqual(self._db_bytes(), corrupt)


class WindowAndVersionIdentity(EvidenceVersionsCase):
    def test_same_fingerprint_changed_raw_same_offset_two_versions_one_vote(self):
        # Falsifier: normalized fingerprint unchanged, raw bytes changed at
        # the same offset -> 2 evidence versions, ONE logical vote (same L).
        first = ev.record_page(self.dir, source=self.source(), start=0,
                               raw=b"widget1 FAIL x=1\n", context=None,
                               observations=[self.obs()])
        second = ev.record_page(self.dir, source=self.source(), start=0,
                                raw=b"widget2 FAIL x=1\n", context=None,
                                observations=[self.obs()])
        self.assertNotEqual(first["window_id"], second["window_id"])
        self.assertNotEqual(first["version_ids"], second["version_ids"])
        self.assertEqual(first["created_counts"]["versions"], 1)
        self.assertEqual(second["created_counts"]["versions"], 1)
        conn = self.open_db()
        keys = [row[0] for row in conn.execute(
            "SELECT DISTINCT logical_key FROM versions")]
        self.assertEqual(len(keys), 1)  # one logical occurrence, two views
        self.assertEqual(conn.execute(
            "SELECT COUNT(*) FROM versions").fetchone()[0], 2)

    def test_exact_replay_creates_nothing_and_is_immutable(self):
        # Falsifier: exact replay is idempotent — no additional facts,
        # timestamps untouched (immutability of first capture).
        args = dict(source=self.source(), start=0, raw=b"record one\n",
                    context={"depth": 2},
                    observations=[self.obs()])
        first = ev.record_page(self.dir, **args)
        first_created = dict(first["created_counts"])
        replay = ev.record_page(self.dir, **args)
        self.assertEqual(replay["window_id"], first["window_id"])
        self.assertEqual(replay["version_ids"], first["version_ids"])
        self.assertEqual(replay["created_counts"],
                         {"windows": 0, "contexts": 0, "versions": 0})
        self.assertNotEqual(first_created, replay["created_counts"])
        conn = self.open_db()
        stamps = conn.execute(
            "SELECT COUNT(DISTINCT created_at) FROM versions"
        ).fetchone()[0]
        self.assertEqual(stamps, 1)

    def test_different_generation_and_geometry_new_window_no_new_vote(self):
        # Falsifier: different g / page geometry creates another evidence
        # view WITHOUT another independent vote — one logical key, both
        # versions hang off it, storage rows are not incident counts.
        base = ev.record_page(self.dir, source=self.source(), start=0,
                              raw=b"body one\n", context=None,
                              observations=[self.obs()])
        moved = ev.record_page(
            self.dir, source=self.source(generation="gen-b2"), start=9,
            raw=b"prefix body one\n", context=None,
            observations=[self.obs()])
        self.assertNotEqual(base["window_id"], moved["window_id"])
        self.assertNotEqual(base["version_ids"], moved["version_ids"])
        self.assertEqual(moved["created_counts"],
                         {"windows": 1, "contexts": 0, "versions": 1})
        summary = ev.read_summary(self.dir)
        self.assertEqual(summary["counts"]["versions"], 2)
        self.assertEqual(summary["counts"]["windows"], 2)
        conn = self.open_db()
        self.assertEqual(conn.execute(
            "SELECT COUNT(DISTINCT logical_key) FROM versions"
        ).fetchone()[0], 1)  # exactly one logical vote

    def test_zero_length_raw_window_retained(self):
        # Falsifier: an empty (zero-byte) consumed page is still a window.
        receipt = ev.record_page(self.dir, source=self.source(), start=0,
                                 raw=b"", context=None, observations=[])
        self.assertEqual(receipt["created_counts"]["windows"], 1)
        conn = self.open_db()
        row = conn.execute(
            "SELECT page_digest, raw_sha256, start, end FROM windows"
        ).fetchone()
        self.assertEqual(row[2], 0)
        self.assertEqual(row[3], 0)
        self.assertEqual(row[1], hashlib.sha256(b"").hexdigest())

    def test_window_recorded_even_without_observation(self):
        # Spec: every consumed page retains its window fact, even when it
        # yields no new logical observation.
        receipt = ev.record_page(self.dir, source=self.source(), start=0,
                                 raw=b"quiet page\n", context=None,
                                 observations=[])
        self.assertEqual(receipt["version_ids"], [])
        self.assertEqual(receipt["created_counts"],
                         {"windows": 1, "contexts": 0, "versions": 0})


class RawByteFidelity(EvidenceVersionsCase):
    def test_invalid_utf8_raw_bytes_hash_exactly(self):
        # Falsifier: invalid UTF-8 page bytes digest correctly (raw hashing,
        # never decode/replace round-trips).
        raw = b"ok\xff\xfe line\x80\n"
        receipt = ev.record_page(self.dir, source=self.source(), start=0,
                                 raw=raw, context=None, observations=[])
        conn = self.open_db()
        stored = conn.execute(
            "SELECT page_digest, raw_sha256 FROM windows").fetchone()
        self.assertEqual(stored[0], hashlib.sha256(raw).hexdigest())
        self.assertEqual(stored[1], stored[0])
        self.assertEqual(receipt["window_id"].startswith("W-"), True)

    def test_oversized_none_raw_is_typed_gap_never_fake_hash(self):
        # Falsifier: closed synthetic oversized page raw=None must be a
        # typed UNKNOWN gap, never a fabricated byte digest.
        receipt = ev.record_page(self.dir, source=self.source(), start=0,
                                 raw=None, context=None, observations=[])
        conn = self.open_db()
        row = conn.execute(
            "SELECT page_digest, raw_sha256 FROM windows").fetchone()
        self.assertEqual(row[0], ev._PAGE_DIGEST_UNKNOWN)
        self.assertIsNone(row[1])
        self.assertNotIn("UNKNOWN", receipt["version_ids"])
        self.assertTrue(any(u["kind"] == "raw-gap"
                            for u in receipt["unknown_evidence"]))
        summary = ev.read_summary(self.dir)
        self.assertEqual(summary["recent_windows"][0]["page_digest"],
                         "UNKNOWN")
        self.assertNotEqual(receipt["window_id"], ev._identity(
            "W-", [self.source()["source_kind"],
                   self.source()["lineage_id"],
                   self.source()["generation"], 0, 0,
                   hashlib.sha256(b"").hexdigest()]))

    def test_no_raw_prompt_or_path_text_in_store(self):
        # Privacy law: stored facts carry no raw page text and never the
        # census project path.
        secret = b"token=hunter2 prompt text\n"
        ev.record_page(self.dir, source=self.source(), start=0, raw=secret,
                       context={"carried_observations": 1},
                       observations=[self.obs()])
        blob = self._db_bytes()
        self.assertNotIn(b"hunter2", blob)
        self.assertNotIn(b"/home/user/proj", blob)
        self.assertNotIn(b"widget-src", blob)


class ReferenceResolution(EvidenceVersionsCase):
    def test_carried_context_known_and_unbound_refs_typed(self):
        # Falsifier: carried context resolves known window refs durably and
        # types unbound refs as UNKNOWN evidence (no invented bytes).
        page1 = ev.record_page(self.dir, source=self.source(), start=0,
                               raw=b"first\n", context=None,
                               observations=[])
        prior_window = page1["window_id"]
        page2 = ev.record_page(
            self.dir, source=self.source(), start=7, raw=b"second\n",
            context=None,
            observations=[self.obs(refs=[
                {"kind": "window", "window_id": prior_window},
                {"kind": "unbound", "unbound_value": "carried-pre-store"},
            ])])
        self.assertEqual(len(page2["version_ids"]), 1)
        unbound = [u for u in page2["unknown_evidence"]
                   if u["kind"] == "unbound-context"]
        self.assertEqual(len(unbound), 1)
        self.assertTrue(unbound[0]["unbound_id"].startswith("U-"))
        conn = self.open_db()
        refs = conn.execute(
            "SELECT ref_json, window_id, unbound_id FROM version_refs"
        ).fetchall()
        self.assertEqual(len(refs), 3)  # own window + 2 caller refs
        joined = "".join(r[0] for r in refs).encode("utf-8")
        self.assertIn(b'"kind":"unbound"', joined)
        unbound_row = [r for r in refs if r[2] is not None]
        self.assertEqual(len(unbound_row), 1)
        self.assertIsNone(unbound_row[0][1])  # unbound refs bind no window
        blob = self._db_bytes()
        self.assertNotIn(b"carried-pre-store", blob)  # value never stored

    def test_missing_window_ref_holds_and_rolls_back_whole_call(self):
        # Falsifier: references never invent prior raw content; a missing
        # window holds the WHOLE call (window+context inserts roll back too).
        ev.record_page(self.dir, source=self.source(), start=0,
                       raw=b"anchor\n", context=None, observations=[])
        before = ev.read_summary(self.dir)
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(
                self.dir, source=self.source(), start=7, raw=b"next\n",
                context={"carried": True},
                observations=[self.obs(refs=[
                    {"kind": "window", "window_id": "W-" + "0" * 64}])])
        after = ev.read_summary(self.dir)
        self.assertEqual(after["counts"], before["counts"])

    def test_digest_contradiction_holds_never_rewrites(self):
        # Falsifier: an expected_digest contradicting the stored window
        # digest holds; stored evidence is never silently reinterpreted.
        page1 = ev.record_page(self.dir, source=self.source(), start=0,
                               raw=b"real bytes\n", context=None,
                               observations=[])
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(
                self.dir, source=self.source(), start=12, raw=b"later\n",
                context=None,
                observations=[self.obs(refs=[
                    {"kind": "window", "window_id": page1["window_id"],
                     "expected_digest": "f" * 64}])])

    def test_contradictory_same_id_payload_holds(self):
        # Falsifier: a colliding identity with a different payload is held,
        # never overwritten (e.g. severity changed under same V inputs).
        args = dict(source=self.source(), start=0, raw=b"same\n",
                    context=None)
        ev.record_page(self.dir, observations=[self.obs()], **args)
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir,
                           observations=[self.obs(severity="low")], **args)
        self.assertEqual(ev.read_summary(self.dir)
                         ["counts"]["versions"], 1)


class LegacyRecords(EvidenceVersionsCase):
    def legacy_row(self, **over):
        base = {"detector_id": "cmd-loop", "detector_version": "det-v2",
                "severity": "medium", "status": "unverified-symptom",
                "source_kind": "log", "generation": "gen-old",
                "span": "bytes@12", "fingerprint": "fp-legacy-1",
                "at": "2026-01-01T00:00:00+00:00",
                "dedupe_key": "v1-legacy", "lineage_id": "42:7"}
        base.update(over)
        return base

    def test_legacy_raw_unavailable_not_fake_hash(self):
        # Falsifier: legacy normalized-record digest is explicitly NOT a
        # raw-source hash; raw evidence is the typed unavailable constant.
        receipt = ev.record_legacy(self.dir, [self.legacy_row()])
        legacy_id = receipt["unknown_evidence"][0]["legacy_id"]
        self.assertTrue(legacy_id.startswith("G-"))
        conn = self.open_db()
        row = conn.execute(
            "SELECT raw_evidence, record_digest FROM legacy_records"
        ).fetchone()
        self.assertEqual(row[0], "unavailable")
        self.assertEqual(row[1], legacy_id[2:])
        summary = ev.read_summary(self.dir)
        self.assertEqual(summary["legacy_raw_unavailable"], 1)

    def test_legacy_replay_idempotent_and_payload_bound_identity(self):
        row = self.legacy_row()
        first = ev.record_legacy(self.dir, [row])
        replay = ev.record_legacy(self.dir, [row])
        self.assertEqual(replay["created_counts"]["legacy_records"], 0)
        self.assertEqual(first["created_counts"]["legacy_records"], 1)
        # Legacy identity IS the payload digest: a changed field is a
        # different serialized record with a different G — a contradiction
        # under one G is structurally impossible, and identity collision
        # can never rewrite a row.
        changed = ev.record_legacy(
            self.dir, [self.legacy_row(severity="high")])
        self.assertEqual(changed["created_counts"]["legacy_records"], 1)
        self.assertNotEqual(
            first["unknown_evidence"][0]["legacy_id"],
            changed["unknown_evidence"][0]["legacy_id"])
        self.assertEqual(ev.read_summary(self.dir)
                         ["counts"]["legacy_records"], 2)

    def test_legacy_arbitrary_body_rejected(self):
        # Closed metadata: no arbitrary body dicts enter the legacy table.
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_legacy(self.dir, [self.legacy_row(extra_field="x")])
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_legacy(self.dir, [{"source_kind": "log"}])


class ValidationAndContextBounds(EvidenceVersionsCase):
    def test_source_closed_fields_and_path_never_stored(self):
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source={"source_kind": "log"},
                           start=0, raw=b"x\n", context=None,
                           observations=[])
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(extra="k"),
                           start=0, raw=b"x\n", context=None,
                           observations=[])

    def test_observation_closed_schema_enforced(self):
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=0,
                           raw=b"x\n", context=None,
                           observations=[self.obs(unknown_key=1)])
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=0,
                           raw=b"x\n", context=None,
                           observations=[self.obs(severity="catastrophic")])
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=-1,
                           raw=b"x\n", context=None, observations=[])
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=0,
                           raw="not bytes", context=None, observations=[])

    def test_context_bounded_no_arbitrary_serialization(self):
        # Falsifier: unbounded or unrepresentable context is refused.
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=0,
                           raw=b"x\n", context={"big": "y" * 600},
                           observations=[])
        with self.assertRaises(ev.EvidenceHeld):
            deep = cur = {}
            for _ in range(6):
                cur["d"] = {}
                cur = cur["d"]
            ev.record_page(self.dir, source=self.source(), start=0,
                           raw=b"x\n", context=deep, observations=[])
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=0,
                           raw=b"x\n", context={"o": object()},
                           observations=[])


class DurabilityAndConcurrency(EvidenceVersionsCase):
    def test_storage_fault_midcall_rolls_back_and_replays_durably(self):
        # Falsifier: a fault after dependency inserts must leave no partial
        # facts, and the replay afterwards must be durable.
        ev.record_page(self.dir, source=self.source(), start=0,
                       raw=b"anchor\n", context=None, observations=[])
        before = ev.read_summary(self.dir)
        original_matches = ev._row_matches

        def flaky(*args, **kwargs):
            raise OSError("simulated storage fault")

        ev._row_matches = flaky
        try:
            with self.assertRaises(ev.EvidenceHeld):
                ev.record_page(self.dir, source=self.source(), start=7,
                               raw=b"next\n", context={"carried": True},
                               observations=[self.obs()])
        finally:
            ev._row_matches = original_matches
        self.assertEqual(ev.read_summary(self.dir)["counts"],
                         before["counts"])
        # Durable replay: the same evidence records cleanly afterwards.
        receipt = ev.record_page(self.dir, source=self.source(), start=7,
                                 raw=b"next\n", context={"carried": True},
                                 observations=[self.obs()])
        self.assertEqual(receipt["created_counts"],
                         {"windows": 1, "contexts": 1, "versions": 1})

    def test_concurrent_writers_dedupe(self):
        # Falsifier: two concurrent writers recording the same evidence
        # dedupe to exactly one window and one version.
        args = dict(source=self.source(), start=0, raw=b"shared page\n",
                    context=None, observations=[self.obs()])
        ev.record_page(self.dir, **args)  # schema exists before the race
        errors = []

        def worker():
            try:
                ev.record_page(self.dir, **args)
            except Exception as exc:  # noqa: BLE001 — worker thread boundary
                errors.append(exc)

        threads = [threading.Thread(target=worker) for _ in range(4)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertEqual(errors, [])
        summary = ev.read_summary(self.dir)
        self.assertEqual(summary["counts"]["windows"], 1)
        self.assertEqual(summary["counts"]["versions"], 1)

    def test_wal_files_coherent_after_calls(self):
        # WAL/SHM coherence: only SQLite manages them; after calls complete
        # (last connection closed) the database is fully readable and the
        # WAL has been checkpointed by SQLite itself.
        ev.record_page(self.dir, source=self.source(), start=0,
                       raw=b"wal check\n", context=None, observations=[])
        self.assertFalse(os.path.exists(self.db_file() + "-wal"))
        conn = self.open_db()
        self.assertEqual(conn.execute(
            "SELECT COUNT(*) FROM windows").fetchone()[0], 1)


class SummaryAndBaseline(EvidenceVersionsCase):
    def test_read_summary_bounded_and_labeled(self):
        for i in range(5):
            ev.record_page(self.dir, source=self.source(), start=i * 10,
                           raw=f"page {i}\n".encode(), context=None,
                           observations=[self.obs(span=f"bytes@{i}")])
        summary = ev.read_summary(self.dir, limit=3)
        self.assertEqual(len(summary["recent_windows"]), 3)
        self.assertEqual(len(summary["recent_versions"]), 3)
        self.assertEqual(summary["counts"]["windows"], 5)
        self.assertEqual(summary["counts"]["versions"], 5)
        self.assertNotIn("incidents", summary)  # never a logical count

    def test_baseline_distinct_occurrences_are_distinct_votes(self):
        # Negative control for the one-vote law: a genuinely different
        # fingerprint at a different offset is a DIFFERENT logical key.
        a = ev.record_page(self.dir, source=self.source(), start=0,
                           raw=b"a fails\n", context=None,
                           observations=[self.obs(span="bytes@0",
                                                  fingerprint="fp-a")])
        b = ev.record_page(self.dir, source=self.source(), start=8,
                           raw=b"tail b fails\n", context=None,
                           observations=[self.obs(span="bytes@8",
                                                  fingerprint="fp-b")])
        self.assertNotEqual(a["version_ids"], b["version_ids"])
        conn = self.open_db()
        self.assertEqual(conn.execute(
            "SELECT COUNT(DISTINCT logical_key) FROM versions"
        ).fetchone()[0], 2)


class ContextIdentityOneSource(EvidenceVersionsCase):
    """EV-CONTEXT-IDENTITY-01: context has one authoritative source per
    observation and every accepted context is identity-bound into C/V."""

    def test_per_observation_context_change_changes_version(self):
        # The finding: changing only observation.context returned the
        # same V. Now the per-observation context is authoritative and
        # bound: same L, different C, different V.
        a = ev.record_page(self.dir, source=self.source(), start=0,
                           raw=b"page one\n", context=None,
                           observations=[self.obs(
                               context={"carried_observations": 1})])
        b = ev.record_page(self.dir, source=self.source(), start=9,
                           raw=b"page two\n", context=None,
                           observations=[self.obs(
                               context={"carried_observations": 2})])
        self.assertNotEqual(a["version_ids"], b["version_ids"])
        conn = self.open_db()
        self.assertEqual(conn.execute(
            "SELECT COUNT(*) FROM contexts "
            "WHERE context_json != 'null'").fetchone()[0], 2)

    def test_two_observations_same_l_different_context_two_versions(self):
        receipt = ev.record_page(
            self.dir, source=self.source(), start=0, raw=b"page\n",
            context=None,
            observations=[self.obs(context={"carried_observations": 1}),
                          self.obs(context={"carried_observations": 2})])
        self.assertEqual(len(set(receipt["version_ids"])), 2)
        conn = self.open_db()
        self.assertEqual(conn.execute(
            "SELECT COUNT(DISTINCT logical_key) FROM versions"
        ).fetchone()[0], 1)  # one logical vote, two evidence versions

    def test_top_level_context_applies_when_observation_omits(self):
        ev.record_page(self.dir, source=self.source(), start=0,
                       raw=b"page\n", context={"carried_observations": 1},
                       observations=[self.obs(context=None)])
        conn = self.open_db()
        rows = conn.execute(
            "SELECT context_json FROM contexts "
            "WHERE context_json != 'null'").fetchall()
        self.assertEqual(rows, [("{\"carried_observations\":1}",)])
        self.assertEqual(len(rows), 1)

    def test_conflicting_context_sources_refused_before_any_io(self):
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(
                self.dir, source=self.source(), start=0, raw=b"page\n",
                context={"carried_observations": 1},
                observations=[self.obs(context={"carried_observations": 2})])
        self.assertFalse(os.path.exists(self.db_file()))


class CarriedSpanBinding(EvidenceVersionsCase):
    """EV-CARRIED-UNKNOWN-02: carried state outside the current page
    needs a durable prior-window ref or an explicit unbound marker."""

    def test_carried_offset_outside_page_held_without_refs(self):
        # The finding: page [100,108) + carried prior_offset=0 was
        # accepted fully bound. Now the current page is never credited
        # for earlier bytes.
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=100,
                           raw=b"current\n", context={"prior_offset": 0},
                           observations=[self.obs(span="bytes@0")])
        summary = ev.read_summary(self.dir)
        self.assertEqual(summary["counts"]["versions"], 0)

    def test_carried_offset_bound_by_durable_prior_window(self):
        prior = ev.record_page(self.dir, source=self.source(), start=0,
                               raw=b"earlier\n", context=None,
                               observations=[])
        receipt = ev.record_page(
            self.dir, source=self.source(), start=100, raw=b"current\n",
            context=None,
            observations=[self.obs(
                span="bytes@0", context={"prior_offset": 0},
                refs=[{"kind": "window",
                       "window_id": prior["window_id"]}])])
        self.assertEqual(len(receipt["version_ids"]), 1)
        self.assertEqual(receipt["unknown_evidence"], [])

    def test_carried_offset_explicit_unbound_accepted(self):
        receipt = ev.record_page(
            self.dir, source=self.source(), start=100, raw=b"current\n",
            context=None,
            observations=[self.obs(
                span="bytes@0", context={"prior_offset": 0},
                refs=[{"kind": "unbound",
                       "unbound_value": "pre-store carried state"}])])
        self.assertTrue(any(u["kind"] == "unbound-context"
                            for u in receipt["unknown_evidence"]))

    def test_prior_window_id_must_be_among_refs(self):
        prior = ev.record_page(self.dir, source=self.source(), start=0,
                               raw=b"earlier\n", context=None,
                               observations=[])
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(
                self.dir, source=self.source(), start=100, raw=b"current\n",
                context=None,
                observations=[self.obs(
                    context={"prior_offset": 0,
                             "prior_window_id": prior["window_id"]})])


class StoredFactIntegrity(EvidenceVersionsCase):
    """EV-FACT-INTEGRITY-03: persisted W/V/C identities and the full V
    dependency graph recompute before a reference or replay is accepted."""

    def _hash_bytes(self):
        with open(self.db_file(), "rb") as fh:
            return fh.read()

    def test_tampered_referenced_window_payload_holds(self):
        # The finding: start->999 plus cached digests f*64 under the old
        # W was accepted when expected_digest repeated the tampered
        # cache. Identity recomputation from the persisted payload holds.
        prior = ev.record_page(self.dir, source=self.source(), start=0,
                               raw=b"earlier\n", context=None,
                               observations=[])
        conn = self.open_db()
        conn.execute(
            "UPDATE windows SET start=999, page_digest=?, raw_sha256=? "
            "WHERE window_id=?", ("f" * 64, "f" * 64, prior["window_id"]))
        conn.commit()
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=100,
                           raw=b"later\n", context=None,
                           observations=[self.obs(refs=[
                               {"kind": "window",
                                "window_id": prior["window_id"],
                                "expected_digest": "f" * 64}])])

    def test_deleted_child_refs_hold_on_replay(self):
        # The finding: deleting all version_refs let an exact replay
        # succeed with zero created counts and a ref-less V. Now holds.
        args = dict(source=self.source(), start=0, raw=b"page\n",
                    context=None)
        ev.record_page(self.dir, observations=[self.obs(refs=[
            {"kind": "unbound", "unbound_value": "carried"}])], **args)
        conn = self.open_db()
        conn.execute("DELETE FROM version_refs")
        conn.commit()
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, observations=[self.obs(refs=[
                {"kind": "unbound", "unbound_value": "carried"}])], **args)

    def test_tampered_context_json_holds_on_replay(self):
        args = dict(source=self.source(), start=0, raw=b"page\n",
                    context={"carried_observations": 1})
        ev.record_page(self.dir, observations=[self.obs()], **args)
        conn = self.open_db()
        conn.execute(
            "UPDATE contexts SET context_json='{\"carried_observations\":9}'")
        conn.commit()
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, observations=[self.obs()], **args)

    def test_tampered_version_payload_holds_on_replay(self):
        args = dict(source=self.source(), start=0, raw=b"page\n",
                    context=None)
        ev.record_page(self.dir, observations=[self.obs()], **args)
        conn = self.open_db()
        conn.execute("UPDATE versions SET fingerprint='fp-tampered'")
        conn.commit()
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, observations=[self.obs()], **args)


class RawContextPrivacy(EvidenceVersionsCase):
    """EV-RAW-CONTEXT-04: the context projection is closed and typed;
    free text never enters the DB, WAL or SHM — structurally, not by
    secret-marker scanning."""

    def test_free_text_context_refused_never_stored(self):
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=0,
                           raw=b"x\n",
                           context={"prompt": "SYNTHETIC-CANARY prompt"},
                           observations=[])
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=0,
                           raw=b"x\n", context=None,
                           observations=[self.obs(
                               context={"note": "SYNTHETIC-CANARY per-obs"})])
        # Refusals happen before any I/O: no store, no sidecars.
        self.assertFalse(os.path.exists(self.db_file()))

    def test_digest_metadata_accepted_body_never_stored(self):
        digest = hashlib.sha256(b"secret carried body").hexdigest()
        ev.record_page(self.dir, source=self.source(), start=0, raw=b"x\n",
                       context={"carried_digest": digest},
                       observations=[self.obs()])
        blob = self._db_bytes()
        self.assertIn(digest.encode(), blob)
        self.assertNotIn(b"secret carried body", blob)


class PreflightReadOnlyBytes(EvidenceVersionsCase):
    """EV-PREFLIGHT-READONLY-05 / EV-UNKNOWN-STORE-06: genuinely
    read-only preflight and write paths that hold without touching
    unknown store or sidecar bytes."""

    def _wal_fixture_via_crash(self):
        # Leave a schema-99 WAL store with live sidecars, exactly as
        # after a synthetic process exit without close.
        script = (
            "import sqlite3, os\n"
            f"conn = sqlite3.connect({self.db_file()!r})\n"
            "conn.execute('PRAGMA journal_mode=WAL')\n"
            "conn.execute('CREATE TABLE meta(key TEXT PRIMARY KEY, "
            "value TEXT NOT NULL)')\n"
            "conn.execute(\"INSERT INTO meta VALUES('schema_version','99')\")\n"
            "conn.commit()\n"
            "os._exit(0)\n"
        )
        subprocess.run([sys.executable, "-c", script], check=True)

    def _hashes(self):
        # DB and WAL are evidence bytes and must stay byte-identical.
        # The -shm is volatile SQLite reader coordination scratch: its
        # read-marks change under any reader, so we assert presence and
        # size only (never removal, never growth of the DB or WAL).
        out = {}
        for suffix in ("", "-wal", "-shm"):
            path = self.db_file() + suffix
            if os.path.exists(path):
                out["exists" + suffix] = True
                if suffix in ("", "-wal"):
                    with open(path, "rb") as fh:
                        out[suffix] = hashlib.sha256(fh.read()).hexdigest()
                else:
                    out["size" + suffix] = os.path.getsize(path)
            else:
                out["exists" + suffix] = False
        return out

    def test_preflight_readonly_leaves_unknown_wal_store_intact(self):
        self._wal_fixture_via_crash()
        before = self._hashes()
        self.assertTrue(before["exists-wal"])
        with self.assertRaises(ev.EvidenceHeld):
            ev.preflight(self.dir)
        self.assertEqual(self._hashes(), before)

    def test_write_path_holds_unknown_wal_store_intact(self):
        self._wal_fixture_via_crash()
        before = self._hashes()
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=0,
                           raw=b"x\n", context=None, observations=[])
        self.assertEqual(self._hashes(), before)

    def test_unknown_no_marker_store_holds_and_write_leaves_unchanged(self):
        conn = self.open_db()
        conn.execute("CREATE TABLE unrelated(x TEXT)")
        conn.execute("INSERT INTO unrelated VALUES('data')")
        conn.commit()
        conn.close()
        before = self._db_bytes()
        with self.assertRaises(ev.EvidenceHeld):
            ev.preflight(self.dir)
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=0,
                           raw=b"x\n", context=None, observations=[])
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_legacy(self.dir, [{
                "detector_id": "cmd-loop", "detector_version": "det-v2",
                "severity": "medium", "status": "unverified-symptom",
                "source_kind": "log", "generation": "gen-old",
                "span": "bytes@12", "fingerprint": "fp-legacy-1",
                "at": "2026-01-01T00:00:00+00:00",
                "dedupe_key": "v1-legacy", "lineage_id": "42:7"}])
        self.assertEqual(self._db_bytes(), before)
        conn = self.open_db()
        self.assertIsNone(conn.execute(
            "SELECT name FROM sqlite_master WHERE name='meta'").fetchone())

    def test_empty_sqlite_file_held_not_blessed(self):
        os.makedirs(self.dir, exist_ok=True)
        with open(self.db_file(), "wb"):
            pass
        before = self._db_bytes()
        with self.assertRaises(ev.EvidenceHeld):
            ev.preflight(self.dir)
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=0,
                           raw=b"x\n", context=None, observations=[])
        self.assertEqual(self._db_bytes(), before)

    def test_future_schema_delete_journal_not_flipped_to_wal(self):
        ev.record_page(self.dir, source=self.source(), start=0,
                       raw=b"seed\n", context=None, observations=[])
        conn = self.open_db()
        conn.execute("PRAGMA journal_mode=DELETE")
        conn.execute("UPDATE meta SET value='99' "
                     "WHERE key='schema_version'")
        conn.commit()
        conn.close()
        before = self._db_bytes()
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=5,
                           raw=b"more\n", context=None, observations=[])
        self.assertEqual(self._db_bytes(), before)
        conn = self.open_db()
        self.assertEqual(
            conn.execute("PRAGMA journal_mode").fetchone()[0], "delete")

    def test_orphan_sidecars_without_store_held(self):
        os.makedirs(self.dir, exist_ok=True)
        with open(self.db_file() + "-wal", "wb") as fh:
            fh.write(b"\xde\xad")
        with self.assertRaises(ev.EvidenceHeld):
            ev.record_page(self.dir, source=self.source(), start=0,
                           raw=b"x\n", context=None, observations=[])
        self.assertFalse(os.path.exists(self.db_file()))

    def test_concurrent_genesis_dedupes(self):
        # Only an atomically proven-absent store may genesis; racing
        # writers dedupe to one published store, one window, one version.
        args = dict(source=self.source(), start=0, raw=b"first page\n",
                    context=None, observations=[self.obs()])
        errors = []

        def worker():
            try:
                ev.record_page(self.dir, **args)
            except Exception as exc:  # noqa: BLE001 — thread boundary
                errors.append(exc)

        threads = [threading.Thread(target=worker) for _ in range(4)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        self.assertEqual(errors, [])
        summary = ev.read_summary(self.dir)
        self.assertEqual(summary["counts"]["windows"], 1)
        self.assertEqual(summary["counts"]["versions"], 1)


class CanonicalEncoding(EvidenceVersionsCase):
    """EV-CANONICAL-ENCODING-07: canonical json.dumps with the default
    ensure_ascii=True; the store L matches the composer v2 logical
    identity on the same six elements, including non-ASCII metadata."""

    def test_canonical_is_ascii_escaped_like_documented_default(self):
        canon = ev._canonical({"k": "λ"})
        self.assertEqual(
            canon, json.dumps({"k": "λ"}, sort_keys=True,
                              separators=(",", ":")))
        self.assertNotIn("λ", canon)  # ensure_ascii=True escapes U+03BB

    def test_store_logical_key_matches_composer_v2_identity(self):
        import composer
        source = self.source()
        span = "bytes@λ"
        fingerprint = "fp-λ"
        receipt = ev.record_page(
            self.dir, source=source, start=0, raw=b"page\n", context=None,
            observations=[self.obs(span=span, fingerprint=fingerprint)])
        conn = self.open_db()
        store_l = conn.execute(
            "SELECT logical_key FROM versions").fetchone()[0]
        composer_l = composer.observation_identity(
            source["source_kind"], source["lineage_id"], span,
            "cmd-loop", "det-v3", fingerprint)
        self.assertEqual(store_l, "L-" + composer_l[len("v2-"):])


if __name__ == "__main__":
    unittest.main()
