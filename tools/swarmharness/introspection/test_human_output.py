#!/usr/bin/env python3
"""Falsifiers for the human output contract (the --json machine contract is
covered byte-for-byte against the pristine module copy).

The default human view must be scan-friendly AND honest: a stale source's
healthy-looking recorded state renders as "last reported", never current
health; running=0 in queue counts is not zero agents; standing local
collection is never a complete fleet view; missing values never fold into 0;
truncated values are marked; provider units are kept verbatim, never
invented; control/ANSI bytes and embedded newlines never escape a cell; and
no raw JSON fragments leak into the human rendering. Every assertion here
drives the real formatter functions and captures stdout; fixtures are typed
and use invented identities (oak / editor@example.test) only.
"""
import contextlib
import importlib
import io
import json
import os
import sys
import time
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from . import adapters, commands, model

NOW = int(time.time()) - 5

# Synthetic credential-shaped fixtures, assembled at RUNTIME from fixed
# invented pieces: no credential-shaped literal exists in this source
# (raw HIGH-shaped literals are unmaskable under the publication gate's
# admitted policy), while the runtime values still match the redactor
# exactly like the reviewed original fixtures did. There are NO real
# credentials here; nothing real is being obfuscated.
FAKE_CREDENTIAL = "".join(("sk-", "abcdefghijklmnop", "1234"))
FAKE_LEAK = "=".join(("api_key", FAKE_CREDENTIAL + " leaked"))

# Where the pristine (pre-change) module copy lives, for the JSON byte
# equality falsifier. Present in this worktree; the structural check runs
# regardless so the machine-contract guarantee never silently lapses.
BASELINE = Path(os.environ.get(
    "YGG_HUMAN_BASELINE", str(HERE.parents[3] / "baseline" / "pristine_introspection")))

DATASETS = ("dreams", "attempts", "services", "schedules", "plans", "events",
            "heartbeat", "findings", "summary", "repairs", "workers",
            "composer", "reviews")


def cov(source, kind, quality="verified", observed_at=None, ttl=120, reason=None):
    return model.coverage(source, "oak", kind,
                          observed_at=model.iso(observed_at) if observed_at else None,
                          received_at=model.iso(NOW), ttl=ttl, quality=quality,
                          reason=reason, collection_mode="standing")


def result_of(frame, path, now=None):
    result, code = model.query(frame, path, {}, now or (NOW + 2))
    return result, code


def render(result, path):
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        adapters._render(result, list(path))
    return buf.getvalue()


def status_frame(summary, coverage, plans=None):
    """A typed frame whose summary row(s) and coverage are exactly the
    fixture under test; everything else is honestly empty."""
    datasets = {k: [] for k in DATASETS}
    datasets["summary"] = list(summary) if isinstance(summary, list) else [summary]
    if plans:
        datasets["plans"] = plans
    return model.frame(model.digest({"fixture": "human"}), model.iso(NOW),
                       coverage, datasets)


STANDING_COVERAGE = [
    cov("queue:oak", "queue", observed_at=NOW - 4, ttl=30),
    cov("queue-metrics:oak", "attempt_history", observed_at=NOW - 40, ttl=30),
    cov("dream-state:oak", "schedule", observed_at=NOW - 11, ttl=60),
    cov("dreamer-ledgers:oak", "attempt_history", observed_at=NOW - 12, ttl=60),
    cov("heartbeat:oak", "heartbeat", observed_at=NOW - 5, ttl=60),
    cov("services:oak", "services", observed_at=NOW - 3, ttl=60),
    cov("timers:oak", "schedule", observed_at=NOW - 3, ttl=300),
    cov("maintenance:oak", "services", observed_at=NOW - 7, ttl=60),
    cov("workers-registry:oak", "workers", observed_at=NOW - 8, ttl=60),
    cov("kernel-ring:oak", "events", observed_at=NOW - 2, ttl=120),
    cov("pressure:oak", "pressure", observed_at=NOW - 5, ttl=60),
    cov("commitments:oak", "task_ledger", observed_at=NOW - 60, ttl=300),
    # the observed trap: a composer source whose observation is 9 days old
    # against a 120s ttl — verified at capture, stale at query time
    cov("composer:oak", "composer", observed_at=NOW - 9 * 86400, ttl=120),
    cov("reviews:oak", "reviews", observed_at=NOW - 6, ttl=120),
    cov("memory-lock:oak", "memory_lock", observed_at=NOW - 9, ttl=120),
]

STANDING_SUMMARY = {"id": "summary:1", "collection_mode": "standing",
                    "dreams": 1, "running": 0, "failed": 0,
                    "workers": 1, "workers_state": "ok",
                    "queue_is_not_whole_compute": True,
                    "services": 2, "services_dynamic_instances": 1,
                    "composer_observations": 2, "composer_state": "ok",
                    "reviews": 0, "reviews_state": "ok",
                    "memory_lock_modules": 1, "memory_lock_state": "ok",
                    "schedules": 3, "events": 5,
                    "heartbeat_planes": 2, "active_findings": 0}


class SwarmStatusHumanView(unittest.TestCase):
    def setUp(self):
        self.frame = status_frame(STANDING_SUMMARY, STANDING_COVERAGE)
        self.result, _ = result_of(self.frame, ("swarm", "status"))
        self.out = render(self.result, ("swarm", "status"))

    def test_default_status_is_not_a_json_dump(self):
        # The parent-observed defect: the summary row printed as a raw JSON
        # object after "total". The human view must render labeled rows.
        self.assertNotIn("{\"", self.out)
        self.assertNotIn("{'", self.out)
        self.assertNotIn("collection_mode", self.out.splitlines()[0])

    def test_sections_carry_counts_state_freshness(self):
        self.assertIn("collection mode=standing", self.out)
        self.assertIn("NOT a complete fleet view", self.out)
        self.assertRegex(self.out, r"work\s+rows=1 running=0 failed=0 freshness=")
        self.assertRegex(self.out, r"composer\s+observations=2 state=ok freshness=STALE")
        self.assertRegex(self.out, r"workers\s+registry_rows=1")
        self.assertRegex(self.out, r"memory-lock\s+modules=1")

    def test_stale_ok_contradiction_is_labeled_last_reported(self):
        # composer_state=ok next to a 9-day-old composer source: the human
        # view must show effective freshness STALE and the last-reported
        # clause, and must nowhere imply the ok state is current.
        self.assertIn("freshness=STALE age=9d00h", self.out)
        self.assertIn("composer_state=ok is the last reported value from a "
                      "STALE source", self.out)
        self.assertIn("does not assert current health", self.out)

    def test_running_zero_is_explicitly_not_zero_agents(self):
        self.assertIn("running=0 is not zero agents", self.out)

    def test_coverage_lines_show_age_ttl_and_reason(self):
        self.assertIn("age=9d00h", self.out)
        self.assertIn("ttl=120s", self.out)
        self.assertIn("observation_expired", self.out)
        unobserved_frame = status_frame(
            {"id": "s:1", "collection_mode": "standing"},
            [cov("x:oak", "queue", quality="unavailable",
                 reason="collection refused (fail closed)")])
        result, _ = result_of(unobserved_frame, ("swarm", "status"))
        out = render(result, ("swarm", "status"))
        self.assertIn("never-observed", out)
        self.assertIn("UNAVAILABLE", out)
        self.assertIn("collection refused (fail closed)", out)


class StatusHonestyEdges(unittest.TestCase):
    def _render_summary(self, summary, coverage):
        result, _ = result_of(status_frame(summary, coverage), ("swarm", "status"))
        return render(result, ("swarm", "status"))

    def test_failclosed_summary_renders_not_collected_not_zero(self):
        out = self._render_summary(
            {"id": "summary:1", "collection_mode": "invalid", "dreams": None,
             "workers": None, "note": "fail-closed capture: no sources were read"},
            [cov("collection:oak", "collection", quality="unavailable",
                 reason="collection refused (fail closed)")])
        self.assertIn("fail-closed", out)
        self.assertIn("rows=not-collected", out)
        self.assertNotIn("rows=0", out)
        self.assertIn("fail-closed capture", out)

    def test_missing_summary_row_is_not_a_healthy_fleet(self):
        result, _ = result_of(status_frame([], STANDING_COVERAGE), ("swarm", "status"))
        self.assertIn("a missing summary is not a healthy fleet",
                      render(result, ("swarm", "status")))

    def test_unknown_state_without_coverage_stays_unknown(self):
        out = self._render_summary(
            {"id": "s", "collection_mode": "configured", "composer_state": "ok"},
            [])
        self.assertIn("freshness is UNKNOWN", out)

    def test_extra_summary_keys_flatten_without_json(self):
        out = self._render_summary(
            {"id": "s", "collection_mode": "configured", "sources_enrolled": 4,
             "future_key": {"nested": [1, {"deep": True}]}}, STANDING_COVERAGE)
        self.assertIn("sources_enrolled=4", out)
        self.assertIn("future_key.nested[0]=1", out)
        self.assertIn("deep=yes", out)
        self.assertNotIn("json", out.lower().replace("--json", ""))

    def test_verified_composer_state_is_not_slandered(self):
        fresh = [c for c in STANDING_COVERAGE if c["source"] != "composer:oak"]
        fresh.append(cov("composer:oak", "composer", observed_at=NOW - 5, ttl=120))
        out = self._render_summary(STANDING_SUMMARY, fresh)
        self.assertIn("composer", out)
        self.assertRegex(out, r"composer\s+observations=2 state=ok freshness=verified")
        self.assertNotIn("last reported value", out)


class PressureHumanView(unittest.TestCase):
    def setUp(self):
        self.plans = [
            {"kind": "funding", "plane": "editor-plane", "host": "oak",
             "quality": "stale", "plane_freshness": "stale",
             "reader_quality": "verified",
             "grants": [{"id": "editor@example.test:main", "grant_units": 125000,
                         "remaining": 987, "status": "active",
                         "ends_at": NOW + 3600},
                        {"id": "grant-no-unit", "grant_units": None,
                         "remaining": None, "status": "active"}],
             "windows": [{"remaining": 90000, "usage": 35, "number": 1,
                          "nextResetTime": "2026-11-07T00:00:00Z"},
                         {"remaining": 10}, {"remaining": 20}, {"remaining": 30},
                         {"remaining": 40}],
             "observed_at": NOW - 9 * 86400, "error": "reader timed out",
             "unknown_reader": True},
            {"kind": "execution", "plane": "queue", "host": "oak",
             "quality": "verified", "counts": {"running": 0, "queued": 2, "done": 17},
             "serve": {"alive": True}, "unknown_reader": False},
            {"kind": "mystery", "plane": "odd", "host": "oak", "quality": "unknown",
             "nested": {"deep": {"list": [1, 2, None]}}, "flag": True, "empty": {}}]
        frame = status_frame({"id": "s", "collection_mode": "standing"},
                             [cov("pressure:oak", "pressure",
                                  observed_at=NOW - 5, ttl=60)], plans=self.plans)
        self.result, _ = result_of(frame, ("swarm", "pressure"))
        self.out = render(self.result, ("swarm", "pressure"))

    def test_provider_windows_render_as_fields_not_json(self):
        self.assertIn("window[0] remaining=90000 usage=35 number=1 "
                      "nextResetTime=2026-11-07T00:00:00Z", self.out)
        self.assertNotIn("{\"", self.out)

    def test_window_overflow_is_declared_not_dropped(self):
        self.assertIn("windows[4..] (1 more; see --json)", self.out)

    def test_units_are_kept_and_never_invented(self):
        self.assertIn("grant_units=125000", self.out)
        # a missing unit stays missing; no unit word is invented for it
        self.assertIn("grant_units=-", self.out)
        self.assertNotIn("tokens", self.out.lower())
        self.assertIn("none is invented", self.out)

    def test_stale_capacity_is_last_reported_not_current(self):
        self.assertIn("freshness=STALE", self.out)
        self.assertIn("LAST REPORTED observation, not current capacity", self.out)
        self.assertIn("reader error: reader timed out", self.out)

    def test_queue_counts_running_zero_is_not_zero_agents(self):
        self.assertIn("counts done=17 queued=2 running=0", self.out)
        self.assertIn("running=0 is not zero agents", self.out)

    def test_unknown_row_flattens_deterministically(self):
        self.assertIn("mystery", self.out)
        self.assertIn("nested.deep.list[0]=1", self.out)
        self.assertIn("nested.deep.list[2]=-", self.out)
        self.assertIn("flag=yes", self.out)
        self.assertIn("empty=empty-object", self.out)


class ComposerHumanView(unittest.TestCase):
    def test_unknown_row_types_flatten_without_json(self):
        items = [
            {"id": "composer-obs:abc", "row_type": "observation", "status": "open",
             "severity": "warning", "detector_id": "race", "detector_version": "2",
             "repeat_count": 3, "last_at": NOW - 15},
            {"id": "composer-cursor:fp1", "row_type": "cursor",
             "source_label": "editor@example.test.json", "cursor": 123,
             "generation": "g1", "lineage_id": "l1", "pending_fps": 0},
            {"id": "composer-state:1", "row_type": "state",
             "state_dir": "/home/oak/.yggterm/swarm/composer",
             "observations_state": "ok", "scanned_state": "ok",
             "outbox_state": "absent", "scanned_lineages": None,
             "outbox_entries": None}]
        frame = model.frame(model.digest({"fixture": "composer"}), model.iso(NOW),
                            [], {"composer": items})
        result, _ = result_of(frame, ("composer", "ls"))
        out = render(result, ("composer", "ls"))
        self.assertNotIn("{\"", out)
        self.assertNotIn("{'", out)
        self.assertRegex(out, r"obs\s+composer-obs:abc\s+status=open")
        self.assertIn("last_age=", out)
        self.assertIn("cursor=123 generation=g1", out)
        self.assertIn("observations_state=ok", out)
        # missing values stay missing ("-"), never fold into 0
        self.assertIn("scanned_lineages=-", out)
        self.assertNotIn("scanned_lineages=0", out)


class HumanBlockerRegressions(unittest.TestCase):
    """Permanent regressions for the four independently confirmed display
    blockers (HUMAN-F01..F04): recorded capture-time words cannot pose as
    current health on a stale retained frame; dynamic keys and summary
    values cannot smuggle control bytes or tabs into the output; bounded
    lists and clipped cells declare what they omit; and an aggregate kind
    reads as its WORST contributing source, never its first."""

    @staticmethod
    def _fresh_cov(kind, source):
        return cov(source, kind, observed_at=NOW - 1, ttl=60)

    @staticmethod
    def _heartbeat_frame(row, coverage):
        return model.frame(model.digest({"fixture": "regress-hb"}), model.iso(NOW),
                           coverage, {k: [] for k in DATASETS} | {"heartbeat": [row]})

    def test_f01_stale_frame_demotes_recorded_current_funding(self):
        row = {"id": "plan:oak", "kind": "funding", "plane": "oak",
               "reader_quality": "verified", "quality": "verified",
               "plane_freshness": "current", "unknown_reader": False,
               "observed_at": NOW - 1, "grants": []}
        frame = status_frame({"id": "s", "collection_mode": "standing"},
                             [self._fresh_cov("pressure", "pressure:oak")],
                             plans=[row])
        result, _ = result_of(frame, ("swarm", "pressure"), now=NOW + 9 * 86400)
        self.assertEqual(result["coverage"][0]["quality"], "stale")
        out = render(result, ("swarm", "pressure"))
        self.assertIn("freshness=STALE", out)
        self.assertNotIn("freshness=current", out)
        self.assertIn("LAST REPORTED", out)
        self.assertIn("reported(current, age 1s, at capture %s)" % result["as_of"], out)

    def test_f01_stale_frame_demotes_recorded_current_heartbeat(self):
        row = {"id": "plane:oak", "row_type": "plane", "plane": "oak",
               "quality": "verified", "freshness": "current",
               "observed_at": NOW - 1, "grants": []}
        frame = self._heartbeat_frame(
            row, [self._fresh_cov("heartbeat", "heartbeat:oak")])
        result, _ = result_of(frame, ("swarm", "heartbeat"), now=NOW + 9 * 86400)
        self.assertEqual(result["coverage"][0]["quality"], "stale")
        out = render(result, ("swarm", "heartbeat"))
        self.assertIn("freshness=STALE", out)
        self.assertNotIn("freshness=current", out)
        self.assertIn("LAST REPORTED", out)

    def test_f02_dynamic_keys_and_summary_values_are_single_line(self):
        row = {"id": "execution:oak", "kind": "execution", "plane": "queue",
               "quality": "verified", "counts": {"running\nFORGED\x1b[2J": 0},
               "serve": None}
        frame = status_frame({"id": "s", "collection_mode": "standing"},
                             [self._fresh_cov("pressure", "pressure:oak")],
                             plans=[row])
        result, _ = result_of(frame, ("swarm", "pressure"))
        out = render(result, ("swarm", "pressure"))
        self.assertNotIn("\x1b", out)
        self.assertNotIn("\nFORGED", out)
        self.assertIn("running FORGED[2J=0", out)
        self.assertEqual(adapters._cell("oak\tFORGED"), "oak FORGED")
        frame = status_frame({"id": "s", "collection_mode": "configured",
                              "composer_state": "ok\nFORGED\x1b[2J"},
                             [self._fresh_cov("composer", "composer:oak")])
        result, _ = result_of(frame, ("swarm", "status"))
        out = render(result, ("swarm", "status"))
        self.assertNotIn("\x1b", out)
        self.assertNotIn("\nFORGED", out)
        self.assertIn("state=ok FORGED[2J", out)

    def test_f03_bounded_lists_declare_omissions_and_clips_are_marked(self):
        grants = [{"id": "g%d" % i, "grant_units": 100, "remaining": 50}
                  for i in range(9)]
        row = {"id": "plan:oak", "kind": "funding", "plane": "oak",
               "reader_quality": "verified", "quality": "verified",
               "plane_freshness": "current", "unknown_reader": False,
               "observed_at": NOW - 1, "available": "A" * 120, "grants": grants}
        frame = status_frame({"id": "s", "collection_mode": "standing"},
                             [self._fresh_cov("pressure", "pressure:oak")],
                             plans=[row])
        result, _ = result_of(frame, ("swarm", "pressure"))
        out = render(result, ("swarm", "pressure"))
        self.assertIn("grants[6..] (3 more; see --json; 9 total)", out)
        self.assertNotIn("g6", out)
        self.assertIn("A" * 77 + "...", out)
        self.assertNotIn("A" * 81, out)
        hb = {"id": "plane:oak", "row_type": "plane", "plane": "oak",
              "quality": "verified", "freshness": "current",
              "observed_at": NOW - 1,
              "grants": [{"id": "g%d" % i, "grant_units": 100, "remaining": 50}
                         for i in range(10)]}
        frame = self._heartbeat_frame(
            hb, [self._fresh_cov("heartbeat", "heartbeat:oak")])
        result, _ = result_of(frame, ("swarm", "heartbeat"))
        out = render(result, ("swarm", "heartbeat"))
        self.assertIn("grants[8..] (2 more; see --json; 10 total)", out)

    def test_f04_aggregate_freshness_is_the_worst_contributing_source(self):
        fresh = cov("composer:oak", "composer", observed_at=NOW - 1, ttl=60)
        stale = cov("composer:other", "composer", observed_at=NOW - 86400, ttl=60)
        frame = status_frame({"id": "s", "collection_mode": "configured",
                              "composer_observations": 2, "composer_state": "ok"},
                             [fresh, stale])
        result, _ = result_of(frame, ("swarm", "status"))
        out = render(result, ("swarm", "status"))
        self.assertRegex(out, r"composer\s+observations=2 state=ok freshness=STALE")
        self.assertNotIn("state=ok freshness=verified", out)
        self.assertIn("composer_state=ok is the last reported value from a "
                      "STALE source", out)
        self.assertIn("(worst of 2 sources)", out)
        frame = status_frame({"id": "s", "collection_mode": "configured",
                              "composer_state": "ok"}, [])
        result, _ = result_of(frame, ("swarm", "status"))
        out = render(result, ("swarm", "status"))
        self.assertIn("state=ok freshness=UNKNOWN", out)


class CellHygiene(unittest.TestCase):
    def test_control_ansi_and_newlines_never_escape_a_cell(self):
        frame = model.frame(model.digest({"fixture": "hygiene"}), model.iso(NOW), [],
                            {"dreams": [], "attempts": [], "services": [], "schedules": [],
                             "plans": [], "events": [], "heartbeat": [], "findings": [],
                             "repairs": [], "workers": [], "composer": [], "reviews": [],
                             "summary": []})
        result, _ = result_of(frame, ("dream", "ls"))
        # two rows: one carrying unicode + control bytes, one carrying a
        # credential-like string (redaction replaces the whole cell by design)
        result["items"] = [
            {"id": "work:queue:q-é", "queue_id": "q-é", "state": "failed",
             "route": "x" * 60,
             "title": "àéïôü ✓ \x1b[31mred\x1b[0m\nsecond\r\nline\x07tail",
             "reported_attempt_count": None, "attempts": [], "repairs": []},
            {"id": "work:queue:q-secret", "queue_id": "q-secret", "state": "failed",
             "route": "r", "title": FAKE_LEAK,
             "reported_attempt_count": 2, "attempts": [], "repairs": []}]
        out = render(result, ("dream", "ls"))
        for banned in ("\x1b", "\x07", "\r"):
            self.assertNotIn(banned, out)
        for line in out.splitlines():
            self.assertTrue(all(ord(ch) >= 32 or ch == "\t" for ch in line),
                            "control byte escaped a cell: %r" % line)
        self.assertIn("àéïôü ✓", out)          # unicode text survives
        self.assertIn("[redacted", out)          # credential-like metadata redacted
        self.assertNotIn(FAKE_CREDENTIAL, out)
        self.assertIn("attempts=unreported", out)  # missing count is not 0
        self.assertIn("...", out)                # truncation is marked
        self.assertEqual(sum("q-é" in l for l in out.splitlines()), 1)

    def test_redaction_fixture_is_synthetic_and_still_discriminates(self):
        # The credential-shaped value exists only as a runtime join of fixed
        # invented pieces (no raw HIGH-shaped literal in source), still
        # matches the redactor, and this test keeps discriminating: a
        # counterfactual formatter WITHOUT redaction leaks the exact value
        # the real formatter replaces.
        self.assertIsNotNone(model._SECRET.search(FAKE_CREDENTIAL))
        self.assertNotIn(FAKE_CREDENTIAL, adapters._cell(FAKE_LEAK, 300))
        self.assertIn("[redacted", adapters._cell(FAKE_LEAK, 300))
        no_redaction = model._CONTROL.sub("", FAKE_LEAK).replace("\n", " ")
        self.assertIn(FAKE_CREDENTIAL, no_redaction)

    def test_clip_marks_and_pad_keeps_columns(self):
        self.assertEqual(adapters._clip("short", 10), "short")
        clipped = adapters._clip("x" * 40, 10)
        self.assertTrue(clipped.endswith("...") and len(clipped) <= 10)
        self.assertEqual(adapters._pad(None, 4), "-   ")
        self.assertEqual(adapters._cell(True), "yes")
        self.assertEqual(adapters._cell(0), "0")
        self.assertEqual(adapters._cell(1.5), "1.5")

    def test_age_text_bounds(self):
        self.assertEqual(adapters._age_text(4), "4s")
        self.assertEqual(adapters._age_text(90), "1m30s")
        self.assertEqual(adapters._age_text(3 * 3600 + 5 * 60), "3h05m")
        self.assertEqual(adapters._age_text(9 * 86400 + 7 * 3600), "9d07h")
        self.assertEqual(adapters._age_text(None), "?")
        self.assertEqual(adapters._age_text(-5), "?")

    def test_loud_freshness(self):
        self.assertEqual(adapters._loud("current"), "current")
        self.assertEqual(adapters._loud("stale"), "STALE")
        self.assertEqual(adapters._loud(None), "UNKNOWN")


class EmptyScopeHonesty(unittest.TestCase):
    def test_empty_verified_scope_is_not_an_idle_fleet(self):
        frame = model.frame(model.digest({"fixture": "empty"}), model.iso(NOW),
                            [cov("queue:oak", "queue", observed_at=NOW - 4, ttl=30)],
                            {k: [] for k in DATASETS})
        result, _ = result_of(frame, ("dream", "ls"))
        out = render(result, ("dream", "ls"))
        self.assertIn("no rows in this query scope", out)
        self.assertIn("not proof the fleet is idle", out)

    def test_missing_summary_is_not_healthy(self):
        frame = model.frame(model.digest({"fixture": "empty2"}), model.iso(NOW),
                            [], {k: [] for k in DATASETS})
        result, _ = result_of(frame, ("swarm", "status"))
        self.assertIn("a missing summary is not a healthy fleet",
                      render(result, ("swarm", "status")))

    def test_check_legend_says_unknown_is_not_a_pass(self):
        frame = model.frame(model.digest({"fixture": "chk"}), model.iso(NOW),
                            [cov("findings:oak", "events", observed_at=NOW - 2, ttl=120)],
                            {k: [] for k in DATASETS} | {"findings": [
                                {"id": "check:funding", "scope": "funding",
                                 "verdict": "unknown", "detail": "no verified plane"}]})
        result, _ = result_of(frame, ("swarm", "check"))
        out = render(result, ("swarm", "check"))
        self.assertIn("unknown is not a pass", out)
        self.assertIn("falsified=assertion falsified", out)


class HumanPathDoesNoIO(unittest.TestCase):
    def test_rendering_touches_no_files_and_no_subprocess(self):
        frame = status_frame(STANDING_SUMMARY, STANDING_COVERAGE)
        result, _ = result_of(frame, ("swarm", "status"))
        guard = mock.patch("builtins.open",
                           side_effect=AssertionError("file IO in human path"))
        subguard = mock.patch("subprocess.run",
                              side_effect=AssertionError("subprocess in human path"))
        with guard, subguard, contextlib.redirect_stdout(io.StringIO()):
            adapters._render(result, ["swarm", "status"])

    def test_parser_and_help_do_no_io(self):
        # help text and examples are local contract data; parsing and
        # rendering help must not read files or run anything
        buf = io.StringIO()
        with mock.patch("builtins.open",
                        side_effect=AssertionError("file IO in parser/help")), \
                contextlib.redirect_stdout(buf):
            with self.assertRaises(SystemExit) as ctx:
                commands.parse(["swarm", "status", "--help"])
        self.assertEqual(ctx.exception.code, 0)
        self.assertIn("Summarize state and observation coverage", buf.getvalue())
        self.assertIn("ygg swarm status", buf.getvalue())       # multiword example
        buf2 = io.StringIO()
        with mock.patch("builtins.open",
                        side_effect=AssertionError("file IO in parser/help")), \
                contextlib.redirect_stdout(buf2):
            path, args, _ = commands.parse(["swarm", "pressure", "--by", "plane",
                                            "--limit", "5"])
        self.assertEqual(path, ("swarm", "pressure"))
        self.assertEqual((args.by, args.limit), ("plane", 5))


class MachineContractUnchanged(unittest.TestCase):
    """--json must stay byte-identical. The strong form compares against the
    pristine pre-change module copy; the structural form (json.dumps of the
    query result) always runs."""

    ROOTS = {}

    @classmethod
    def setUpClass(cls):
        cls.frame = status_frame(STANDING_SUMMARY, STANDING_COVERAGE,
                                 plans=[{"kind": "execution", "plane": "queue",
                                         "host": "oak", "quality": "verified",
                                         "counts": {"running": 0}, "serve": None,
                                         "unknown_reader": False}])
        cls.root = Path(os.environ.get("YGG_HUMAN_TEST_STORE",
                                       str(Path(os.environ.get("TMPDIR", "/tmp"))
                                           / "ygg-human-test-store")))
        snapshots = cls.root / "snapshots"
        snapshots.mkdir(parents=True, exist_ok=True)
        (snapshots / (cls.frame["snapshot_id"] + ".json")).write_text(
            json.dumps(cls.frame))

    def _serve(self, module, path, args):
        # Pin the reader clock: query_as_of and assessed age_seconds must be
        # identical across calls, or byte comparison would race the wall clock.
        buf = io.StringIO()
        with mock.patch("time.time", return_value=NOW + 2), \
                contextlib.redirect_stdout(buf):
            code = module.serve(path, dict(args), root=str(self.root))
        return buf.getvalue(), code

    def test_json_output_is_json_dumps_of_the_query_result(self):
        out, serve_code = self._serve(adapters, ("swarm", "status"), {"json": True})
        parsed = json.loads(out)
        now = model.epoch(parsed["query_as_of"])
        result, query_code = model.query(self.frame, ("swarm", "status"), {}, now)
        self.assertEqual(serve_code, query_code)
        self.assertEqual(parsed, result)                      # semantic
        self.assertEqual(out, json.dumps(result, indent=1) + "\n")  # byte

    def _pristine(self):
        if not (BASELINE / "adapters.py").exists():
            self.skipTest("pristine baseline copy not present")
        if str(BASELINE.parent) not in sys.path:
            sys.path.insert(0, str(BASELINE.parent))
        return importlib.import_module("pristine_introspection.adapters")

    def test_json_bytes_identical_to_pristine_module(self):
        old = self._pristine()
        for path in (("swarm", "status"), ("swarm", "pressure"), ("dream", "ls")):
            new_out, new_code = self._serve(adapters, path, {"json": True})
            old_out, old_code = self._serve(old, path, {"json": True})
            self.assertEqual(new_out, old_out, path)
            self.assertEqual(new_code, old_code, path)

    def test_human_bytes_intentionally_differ_from_pristine(self):
        # guard against a future edit that accidentally makes the human view
        # a passthrough again (the fix's whole point)
        old = self._pristine()
        new_out, _ = self._serve(adapters, ("swarm", "status"), {})
        old_out, _ = self._serve(old, ("swarm", "status"), {})
        self.assertIn("freshness=", new_out)
        self.assertNotIn("freshness=", old_out)
        self.assertNotEqual(new_out, old_out)


class ScheduleTruncationRegressions(unittest.TestCase):
    def test_native_timer_text_marks_truncation(self):
        value = "Fri 2026-11-06 14:00:00 UTC 3h"
        self.assertTrue(adapters._time_cell(value).endswith("..."))

    def test_schedule_counts_and_authority_mark_truncation(self):
        with contextlib.redirect_stdout(io.StringIO()) as stream:
            adapters._schedule_rows([
                {"id": "plan:o", "kind": "plan", "target": 1234567890123,
                 "remaining": 1234567890123, "staged": 1234567890123},
                {"id": "commitment:o", "kind": "commitment",
                 "revision": "r" * 24, "authority": "a" * 24}])
        out = stream.getvalue()
        self.assertIn("target=1234567...", out)
        self.assertIn("remaining=1234567...", out)
        self.assertIn("rev=" + "r" * 13 + "...", out)
        self.assertIn("authority=" + "a" * 13 + "...", out)


if __name__ == "__main__":
    unittest.main()
