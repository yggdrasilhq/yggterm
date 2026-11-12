#!/usr/bin/env python3
"""Hermetic regression tests for the Claude window, accounting, error and compaction rules in
context-relay-gauge.py. The contract is docs/spec-claude-context-calibration.md.

WHAT THIS FILE PINS
    A. Error recognition. A native refusal row carrying the API's overflow text is a FAULT and a
       HOLD. The CLI's marker `isApiErrorMessage` decides whenever the key is present: only `true`
       is a refusal, and an explicit `false` or an invalid value is an ordinary row, even on a
       `<synthetic>` model. The `<synthetic>` model is a fallback for rows whose key is absent.
       The same phrase quoted inside an ordinary successful report is NOT a fault. A later normal
       turn clears the fault.
    B. Unknown capacity. Observed usage is a lower bound on capacity. It can refute a 200k
       assumption but never asserts a window size. An unknown window has no percentage, no band,
       and no handover. A `[1m]` selector in settings or environment is configuration, not
       evidence about the session: it never asserts a window, and a 200k assumption it may
       contradict is UNVERIFIED.
    C. Coherence. Compaction configured ON publishes an advisory verdict (never LAND or CRITICAL)
       with the raw band kept beside it, relay_recommended false, and text that names the
       conditional override of the legacy relay-at-70% nudge. Configured-off and env-disabled keep
       the verified-capacity relay.
    D. Accounting. Claude usage is additive: input + cache_read + cache_creation, taken from each
       request's own usage block.

HOW TO RUN
    python3 -B -I tests/test_gauge_claude_window_and_compaction.py
    CONTEXT_RELAY_GAUGE=/abs/path/to/other/copy.py python3 -B -I tests/test_gauge_claude_window_and_compaction.py

    Nothing here calls a model, reads a real transcript, or touches the real home directory:
    every hook run gets a fresh temporary HOME and a minimal environment. All token counts,
    session ids and quoted text are invented.
"""
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
GAUGE = Path(os.environ.get("CONTEXT_RELAY_GAUGE") or HERE.parent / "context-relay-gauge.py")
SESSION = "00000000-0000-4000-8000-000000000001"
HAIKU = "claude-haiku-5-5"
RITUAL_MARK = "THE CONTEXT RITUAL"
OVERRIDE_MARK = "OVERRIDES any standing relay-at-70% nudge"
ZERO_USAGE = {"input_tokens": 0, "cache_read_input_tokens": 0,
              "cache_creation_input_tokens": 0, "output_tokens": 0}


def assistant_row(model, input_tokens, cache_read=0, cache_write=0, text="ok"):
    """A Claude transcript assistant row. The usage numbers are invented."""
    return {"type": "assistant", "message": {
        "model": model,
        "content": [{"type": "text", "text": text}],
        "usage": {"input_tokens": input_tokens,
                  "cache_read_input_tokens": cache_read,
                  "cache_creation_input_tokens": cache_write,
                  "output_tokens": 812}}}


def overflow_row():
    """A synthetic refusal row: model `<synthetic>`, zero usage, the API's overflow text."""
    return {"type": "assistant", "message": {
        "model": "<synthetic>",
        "content": [{"type": "text", "text": "Prompt is too long"}],
        "usage": dict(ZERO_USAGE)}}


_ABSENT = object()


def synthetic_row(text, marker=_ABSENT):
    """A `<synthetic>` assistant row with zero usage. `marker` is written as the
    `isApiErrorMessage` value when given, and the key is omitted entirely when it is not.
    The text and the marker values are invented."""
    row = {"type": "assistant", "message": {
        "model": "<synthetic>",
        "content": [{"type": "text", "text": text}],
        "usage": dict(ZERO_USAGE)}}
    if marker is not _ABSENT:
        row["isApiErrorMessage"] = marker
    return row


def api_error_row():
    """A native API error record: the CLI's own `isApiErrorMessage` discriminator, zero usage."""
    return {"type": "assistant", "isApiErrorMessage": True, "message": {
        "model": HAIKU,
        "content": [{"type": "text", "text": "Prompt is too long"}],
        "usage": dict(ZERO_USAGE)}}


def quoted_report_row():
    """An ordinary successful report that QUOTES the overflow string. No native marker at all.
    Usage is 207,989 tokens, the same shape as a healthy session at about 20.8% of 1M."""
    return {"type": "assistant", "message": {
        "model": HAIKU,
        "content": [{"type": "text", "text": 'The status line reads "Prompt is too long" only as a quote.'}],
        "usage": {"input_tokens": 7_989, "cache_read_input_tokens": 200_000,
                  "cache_creation_input_tokens": 0, "output_tokens": 812}}}


def run_hook(rows, *, settings=None, env=None, report=False,
             transcript_name="transcript.jsonl"):
    """Run the gauge the way the hook does: payload on stdin, fresh HOME, minimal env.

    Returns (completed_process, published_json_or_None). `settings=None` writes no
    settings file at all, which is the "nothing measured" case."""
    with tempfile.TemporaryDirectory() as home:
        home_path = Path(home)
        if settings is not None:
            (home_path / ".claude").mkdir()
            (home_path / ".claude" / "settings.json").write_text(json.dumps(settings))
        transcript = home_path / transcript_name
        transcript.write_text("".join(json.dumps(r) + "\n" for r in rows))
        payload = {"session_id": SESSION, "transcript_path": str(transcript)}
        child_env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": home}
        child_env.update(env or {})
        argv = [sys.executable, "-B", "-I", str(GAUGE)] + (["--report"] if report else [])
        proc = subprocess.run(argv, input=json.dumps(payload), capture_output=True,
                              text=True, env=child_env, timeout=60)
        published = None
        pub = home_path / ".claude" / "context-gauge" / f"{SESSION}.json"
        if pub.exists():
            published = json.loads(pub.read_text())
        return proc, published


def load_gauge_module():
    """Import the gauge without running it. Its constants read HOME at import time."""
    spec = importlib.util.spec_from_file_location("gauge_under_test", GAUGE)
    module = importlib.util.module_from_spec(spec)
    with tempfile.TemporaryDirectory() as home, patch.dict(os.environ, {"HOME": home}):
        spec.loader.exec_module(module)
    return module


def usage_of(rows):
    """(used, observed_max, dead, model) that read_usage returns for these transcript rows."""
    gauge = load_gauge_module()
    with tempfile.TemporaryDirectory() as folder:
        path = Path(folder) / "transcript.jsonl"
        path.write_text("".join(json.dumps(r) + "\n" for r in rows))
        return gauge.read_usage(str(path))


class WindowTable(unittest.TestCase):
    def setUp(self):
        self._env = patch.dict(os.environ)
        self._env.start()
        self.addCleanup(self._env.stop)
        for name in ("ANTHROPIC_MODEL", "CLAUDE_MODEL"):
            os.environ.pop(name, None)
        self._home = tempfile.TemporaryDirectory()
        self.addCleanup(self._home.cleanup)
        self.gauge = load_gauge_module()
        self.gauge.HOME = Path(self._home.name)  # no settings.json: nothing to read

    def test_measured_haiku_5_5_id_is_1m_and_known(self):
        # Native CLI measurement: modelUsage["claude-haiku-5-5"].contextWindow == 1000000.
        self.assertEqual(self.gauge.window_for(190_003, HAIKU), (1_000_000, False))
        self.assertEqual(self.gauge.window_for(900_000, HAIKU), (1_000_000, False))

    def test_measured_id_with_a_client_selector_is_still_1m(self):
        self.assertEqual(self.gauge.window_for(190_003, HAIKU + "[1m]"), (1_000_000, False))

    def test_bare_family_tokens_never_assert_a_200k_window(self):
        for model in ("claude-haiku", "claude-haiku-6-0", "claude-sonnet-5-5", "claude-sonnet-4-5"):
            with self.subTest(model=model):
                window, assumed = self.gauge.window_for(190_003, model)
                self.assertTrue(assumed, f"{model} was asserted as a known {window}-token window")

    def test_a_longer_id_that_shares_a_prefix_is_not_the_measured_id(self):
        _, assumed = self.gauge.window_for(0, "claude-haiku-5-50")
        self.assertTrue(assumed)

    def test_observed_usage_never_proves_a_window_size(self):
        # Usage is a LOWER BOUND on capacity. 300k, 900k or 1.5M seen on an unknown model says
        # the window is at least that big, and nothing about its exact size.
        for observed in (300_000, 900_000, 1_500_000):
            with self.subTest(observed=observed):
                self.assertEqual(self.gauge.window_for(observed, "claude-haiku-6-0"), (None, True))

    def test_observed_usage_can_refute_a_200k_assumption_but_not_assert_a_size(self):
        self.assertEqual(self.gauge.window_for(190_000, "claude-opus-4-1"), (200_000, False))
        self.assertEqual(self.gauge.window_for(250_000, "claude-opus-4-1"), (None, True))

    def test_existing_measured_windows_are_unchanged(self):
        self.assertEqual(self.gauge.window_for(0, "claude-fable-5-2"), (1_000_000, False))
        self.assertEqual(self.gauge.window_for(0, "claude-opus-5-5"), (1_000_000, False))
        self.assertEqual(self.gauge.window_for(0, "claude-opus-4-1"), (200_000, False))
        self.assertEqual(self.gauge.window_for(0, "glm-5.3-flash[max]"), (2_000_000, False))

    def _write_settings(self, payload):
        (self.gauge.HOME / ".claude").mkdir(exist_ok=True)
        (self.gauge.HOME / ".claude" / "settings.json").write_text(json.dumps(payload))

    def test_a_stale_unrelated_settings_selector_cannot_grant_unknown_capacity(self):
        # Review control R1-02: an unknown Haiku 6 at 900k, with an unrelated Opus 4 [1m] selector.
        self._write_settings({"model": "claude-opus-4-1[1m]"})
        self.assertEqual(self.gauge.window_for(900_000, "claude-haiku-6"), (None, True))

    def test_a_stale_unrelated_environment_selector_cannot_grant_unknown_capacity(self):
        for name in ("ANTHROPIC_MODEL", "CLAUDE_MODEL"):
            with self.subTest(name=name):
                os.environ[name] = "claude-opus-4-1[1m]"
                try:
                    self.assertEqual(self.gauge.window_for(900_000, "claude-sonnet-6"), (None, True))
                finally:
                    os.environ.pop(name, None)

    def test_a_selector_in_the_settings_env_block_cannot_grant_unknown_capacity(self):
        self._write_settings({"env": {"ANTHROPIC_MODEL": "claude-opus-4-1[1m]"}})
        self.assertEqual(self.gauge.window_for(900_000, "claude-haiku-6"), (None, True))

    def test_a_same_model_selector_is_configured_not_measured(self):
        # A matching [1m] preference is still only a preference: a command-line --model override can
        # make it stale. Opus 4 at 190k must not become a 200k wall, and must not become 1M either.
        self._write_settings({"model": "claude-opus-4-1[1m]"})
        self.assertEqual(self.gauge.window_for(190_000, "claude-opus-4-1"), (None, True))
        self.assertEqual(self.gauge.window_for(900_000, "claude-opus-4-1"), (None, True))

    def test_a_selector_never_moves_a_measured_window(self):
        self._write_settings({"model": "claude-opus-4-1[1m]"})
        self.assertEqual(self.gauge.window_for(190_003, HAIKU), (1_000_000, False))
        self.assertEqual(self.gauge.window_for(0, "claude-fable-5-2"), (1_000_000, False))
        self.assertEqual(self.gauge.window_for(0, "claude-opus-5-5"), (1_000_000, False))
        self.assertEqual(self.gauge.window_for(0, "glm-5.3-flash[max]"), (2_000_000, False))

    def test_without_a_selector_opus_4_keeps_its_measured_200k(self):
        # The control for the two same-model tests above: only the selector differs, not the table.
        self._write_settings({"autoCompactEnabled": True})
        self.assertEqual(self.gauge.window_for(190_000, "claude-opus-4-1"), (200_000, False))


class CompactionPolicy(unittest.TestCase):
    def setUp(self):
        self.gauge = load_gauge_module()

    def policy(self, settings, environ=None):
        return self.gauge.claude_compaction_policy(settings=settings, environ=environ or {})

    def test_configured_on_is_reported_as_configured(self):
        self.assertEqual(self.policy({"autoCompactEnabled": True})[0], "configured-on")

    def test_configured_off(self):
        self.assertEqual(self.policy({"autoCompactEnabled": False})[0], "configured-off")

    def test_absent_setting_is_unknown_not_off(self):
        self.assertEqual(self.policy({})[0], "unknown")

    def test_non_boolean_setting_is_unknown(self):
        self.assertEqual(self.policy({"autoCompactEnabled": "false"})[0], "unknown")

    def test_a_disabling_variable_overrides_configured_on(self):
        for name in ("DISABLE_AUTO_COMPACT", "DISABLE_COMPACT"):
            with self.subTest(name=name):
                state, detail = self.policy({"autoCompactEnabled": True}, {name: "1"})
                self.assertEqual((state, detail), ("env-disabled", name))

    def test_a_disabling_variable_in_the_settings_env_block_counts(self):
        state, _ = self.policy({"autoCompactEnabled": True, "env": {"DISABLE_COMPACT": "true"}})
        self.assertEqual(state, "env-disabled")

    def test_a_zero_value_does_not_disable(self):
        self.assertEqual(self.policy({"autoCompactEnabled": True}, {"DISABLE_AUTO_COMPACT": "0"})[0],
                         "configured-on")

    def test_a_switch_name_that_does_not_exist_is_ignored(self):
        self.assertEqual(
            self.policy({"autoCompactEnabled": True}, {"CLAUDE_CODE_DISABLE_AUTOCOMPACT": "1"})[0],
            "configured-on")


class HookOnClaudeSessions(unittest.TestCase):
    def test_haiku_5_5_session_at_190k_is_not_reported_as_a_wall(self):
        rows = [assistant_row(HAIKU, 3, cache_read=180_000, cache_write=10_000)]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": True})
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(proc.stdout, "", "a 19% reading of a 1M window stays silent below NOTICE")
        self.assertEqual(pub["window"], 1_000_000)
        self.assertFalse(pub["window_assumed"])
        self.assertEqual(pub["pct"], 19)
        self.assertEqual(pub["pct_exact"], 19.0)
        self.assertEqual(pub["verdict"], "OK")
        self.assertEqual(pub["model"], HAIKU)
        for key in ("session", "pct", "used", "window", "window_assumed", "model",
                    "verdict", "dead", "cwd", "pressure_band", "fault", "action",
                    "relay_recommended", "compaction", "observed_max"):
            self.assertIn(key, pub, f"published contract lost key {key!r}")

    def test_report_names_the_measured_window(self):
        rows = [assistant_row(HAIKU, 3, cache_read=180_000, cache_write=10_000)]
        proc, _ = run_hook(rows, settings={"autoCompactEnabled": True}, report=True)
        self.assertIn("of the 1M window", proc.stdout)
        self.assertIn("window known", proc.stdout)
        self.assertIn("advisory only at 70%", proc.stdout)

    def test_unverified_family_is_never_a_handover_command(self):
        rows = [assistant_row("claude-haiku-6-0", 3, cache_read=180_000, cache_write=10_000)]
        # Even with native compaction configured OFF, an unmeasured window commands nothing.
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": False})
        self.assertIn("WINDOW UNVERIFIED", proc.stdout)
        for forbidden in ("LAND THIS SESSION", "THE WALL IS CLOSE", "Take no new work"):
            self.assertNotIn(forbidden, proc.stdout)
        self.assertEqual(pub["verdict"], "UNKNOWN")
        self.assertTrue(pub["window_assumed"])
        self.assertIsNone(pub["window"])
        self.assertIsNone(pub["pct"])
        self.assertIsNone(pub["pressure_band"])
        self.assertFalse(pub["relay_recommended"])

    def test_unverified_small_usage_stays_silent(self):
        rows = [assistant_row("claude-sonnet-5-5", 40_000)]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": True})
        self.assertEqual(proc.stdout, "")
        self.assertEqual(pub["verdict"], "UNKNOWN")

    def test_native_compaction_on_at_critical_claims_neither_a_wall_nor_no_compaction(self):
        rows = [assistant_row(HAIKU, 3, cache_read=899_000)]  # 899,003 of 1M = 90%
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": True})
        self.assertIn("configured on", proc.stdout)
        for forbidden in ("no compaction is armed", "NOT SURVIVABLE", "THE WALL IS CLOSE",
                          "LAND THIS SESSION", "Take no new work"):
            self.assertNotIn(forbidden, proc.stdout)
        self.assertIn(RITUAL_MARK, proc.stdout, "the ritual still rides every message")
        # The raw band is kept; the action verdict is advisory and never CRITICAL.
        self.assertEqual(pub["pressure_band"], "CRITICAL")
        self.assertEqual(pub["verdict"], "NOTICE")
        self.assertEqual(pub["action"], "advisory")
        self.assertFalse(pub["relay_recommended"])
        self.assertEqual(pub["compaction"], "configured-on")

    def test_native_compaction_on_at_land_commands_no_relay(self):
        rows = [assistant_row(HAIKU, 3, cache_read=719_000)]  # 72%
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": True})
        self.assertIn("does NOT command a relay", proc.stdout)
        self.assertNotIn("LAND THIS SESSION", proc.stdout)
        self.assertEqual(pub["pressure_band"], "LAND")
        self.assertEqual(pub["verdict"], "NOTICE")
        self.assertEqual(pub["action"], "advisory")
        self.assertFalse(pub["relay_recommended"])

    def test_native_compaction_on_at_notice_stays_advisory(self):
        rows = [assistant_row(HAIKU, 3, cache_read=599_000)]  # 60%
        proc, _ = run_hook(rows, settings={"autoCompactEnabled": True})
        self.assertIn("No handover is commanded", proc.stdout)
        self.assertNotIn("HANDOVER, DONE", proc.stdout)
        self.assertNotIn("relay boundary", proc.stdout)

    def test_a_disabling_variable_keeps_the_handover_and_names_the_switch(self):
        rows = [assistant_row(HAIKU, 3, cache_read=899_000)]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": True},
                             env={"DISABLE_AUTO_COMPACT": "1"})
        self.assertIn("DISABLE_AUTO_COMPACT is set", proc.stdout)
        self.assertIn("THE WALL IS CLOSE", proc.stdout)
        self.assertEqual(pub["compaction"], "env-disabled")
        self.assertEqual(pub["verdict"], "CRITICAL")
        self.assertEqual(pub["action"], "relay")
        self.assertTrue(pub["relay_recommended"])

    def test_configured_off_keeps_the_handover_and_names_the_setting(self):
        rows = [assistant_row(HAIKU, 3, cache_read=719_000)]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": False})
        self.assertIn("LAND THIS SESSION", proc.stdout)
        self.assertIn("configured OFF", proc.stdout)
        self.assertEqual(pub["compaction"], "configured-off")
        self.assertEqual(pub["verdict"], "LAND")
        self.assertTrue(pub["relay_recommended"])

    def test_unset_policy_is_reported_as_unknown_not_as_off(self):
        rows = [assistant_row(HAIKU, 3, cache_read=899_000)]
        proc, pub = run_hook(rows, settings={})
        self.assertIn("not set in settings", proc.stdout)
        self.assertNotIn("no compaction is armed", proc.stdout)
        self.assertNotIn("LAND THIS SESSION", proc.stdout)
        self.assertEqual(pub["compaction"], "unknown")
        self.assertFalse(pub["relay_recommended"])

    def test_a_missing_settings_file_is_unknown(self):
        rows = [assistant_row(HAIKU, 3, cache_read=180_000, cache_write=10_000)]
        proc, pub = run_hook(rows, settings=None)
        self.assertEqual(proc.stdout, "")
        self.assertEqual(pub["compaction"], "unknown")

    def test_a_switch_name_that_does_not_exist_is_ignored_by_the_hook(self):
        rows = [assistant_row(HAIKU, 3, cache_read=899_000)]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": True},
                             env={"CLAUDE_CODE_DISABLE_AUTOCOMPACT": "1"})
        self.assertEqual(pub["compaction"], "configured-on")
        self.assertNotIn("is set in the environment", proc.stdout)

    def test_post_compaction_usage_is_the_last_row_not_the_peak(self):
        rows = [assistant_row(HAIKU, 3, cache_read=899_000),
                assistant_row(HAIKU, 3, cache_read=119_997)]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": True})
        self.assertEqual(pub["used"], 120_000)
        self.assertEqual(pub["pct"], 12)
        self.assertEqual(proc.stdout, "")

    def test_zcode_rollout_keeps_its_legacy_handover_text(self):
        row = {"type": "model_io", "sessionId": "sess_fixture",
               "model": {"modelId": "glm-5.3-flash", "variant": "max"},
               "response": {"usage": {"inputTokens": 3_000, "cacheReadTokens": 1_497_000,
                                      "cacheWriteTokens": 0}}}
        proc, pub = run_hook([row], transcript_name="model-io-sess_fixture.jsonl")
        self.assertIn("LAND THIS SESSION", proc.stdout)
        self.assertIn("2M window", proc.stdout)
        self.assertNotIn("Native auto-compaction", proc.stdout)
        self.assertEqual(pub["window"], 2_000_000)
        self.assertIsNone(pub.get("compaction"))
        self.assertEqual(pub["action"], "relay")


class ErrorRecognition(unittest.TestCase):
    """A. A quoted phrase is prose. A native refusal row is a fault."""

    def test_quoted_success_report_is_never_dead(self):
        # Discriminates the pre-fix script, which read this row as an overflow.
        rows = [quoted_report_row()]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": True})
        self.assertFalse(pub["dead"])
        self.assertIsNone(pub["fault"])
        self.assertEqual(pub["used"], 207_989)
        self.assertEqual(pub["pct_exact"], 20.8)
        self.assertEqual(pub["verdict"], "OK")
        self.assertEqual(proc.stdout, "", "a healthy 20.8% session stays silent")
        self.assertNotIn("CONTEXT OVERFLOW", proc.stdout)

    def test_quoted_phrase_at_high_usage_is_advisory_not_a_fault(self):
        row = quoted_report_row()
        row["message"]["usage"]["cache_read_input_tokens"] = 899_000  # about 90% of 1M
        proc, pub = run_hook([row], settings={"autoCompactEnabled": True})
        self.assertFalse(pub["dead"])
        self.assertEqual(pub["action"], "advisory")
        self.assertIn("configured on", proc.stdout)
        self.assertNotIn("CONTEXT OVERFLOW", proc.stdout)
        self.assertNotIn("HOLD: do not retry", proc.stdout)

    def test_a_genuine_synthetic_refusal_is_a_fault_and_a_hold(self):
        rows = [assistant_row(HAIKU, 3, cache_read=49_997), overflow_row()]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": True})
        self.assertIn("CONTEXT OVERFLOW", proc.stdout)
        self.assertIn("HOLD", proc.stdout)
        self.assertTrue(pub["dead"])
        self.assertEqual(pub["fault"], "native-overflow")
        self.assertEqual(pub["verdict"], "FAULT")
        self.assertEqual(pub["action"], "hold")
        self.assertFalse(pub["relay_recommended"])

    def test_a_native_api_error_record_is_a_fault_and_a_hold(self):
        rows = [assistant_row(HAIKU, 3, cache_read=49_997), api_error_row()]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": False})
        self.assertIn("CONTEXT OVERFLOW", proc.stdout)
        self.assertTrue(pub["dead"])
        self.assertEqual(pub["verdict"], "FAULT")
        self.assertNotIn("LAND THIS SESSION", proc.stdout)

    def test_a_later_successful_turn_clears_the_fault(self):
        rows = [overflow_row(), assistant_row(HAIKU, 3, cache_read=49_997)]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": True})
        self.assertFalse(pub["dead"])
        self.assertIsNone(pub["fault"])
        self.assertNotEqual(pub["verdict"], "FAULT")
        self.assertEqual(proc.stdout, "")

    def test_a_refusal_does_not_erase_the_last_real_usage(self):
        rows = [assistant_row(HAIKU, 3, cache_read=899_000), overflow_row()]
        _, pub = run_hook(rows, settings={"autoCompactEnabled": True})
        self.assertEqual(pub["used"], 899_003, "a zero-usage refusal must not read as empty")

    def test_high_usage_fault_shows_only_the_hold(self):
        rows = [assistant_row(HAIKU, 3, cache_read=899_000), overflow_row()]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": True})
        self.assertIn("CONTEXT OVERFLOW", proc.stdout)
        for forbidden in ("Keep working", "Keep reads", "THE WALL IS CLOSE", "LAND THIS SESSION",
                          "does NOT command a relay", "advisory only"):
            self.assertNotIn(forbidden, proc.stdout)
        self.assertEqual(pub["pressure_band"], "CRITICAL", "the raw band survives a fault")
        self.assertEqual(pub["verdict"], "FAULT")
        self.assertFalse(pub["relay_recommended"])

    def test_a_fault_with_no_prior_usage_is_still_published(self):
        # Before R2 this printed the HOLD and wrote nothing, so the booter could not see `dead`.
        proc, pub = run_hook([overflow_row()], settings={"autoCompactEnabled": True})
        self.assertIn("CONTEXT OVERFLOW", proc.stdout)
        self.assertIsNotNone(pub, "a fault must be published even with zero usage")
        self.assertTrue(pub["dead"])
        self.assertEqual(pub["used"], 0)
        self.assertIsNone(pub["pct"])

    def test_a_non_overflow_refusal_is_not_an_overflow(self):
        note = {"type": "assistant", "message": {
            "model": "<synthetic>",
            "content": [{"type": "text", "text": "[Request interrupted by user]"}],
            "usage": dict(ZERO_USAGE)}}
        rows = [assistant_row(HAIKU, 3, cache_read=49_997), note]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": True})
        self.assertFalse(pub["dead"])
        self.assertNotIn("CONTEXT OVERFLOW", proc.stdout)

    def test_a_native_refusal_row_is_recognised_by_its_marker_not_by_position(self):
        self.assertEqual(usage_of([quoted_report_row()])[2], False)
        self.assertEqual(usage_of([overflow_row()])[2], True)
        self.assertEqual(usage_of([api_error_row()])[2], True)

    def test_an_explicit_false_marker_on_a_synthetic_row_quoting_the_overflow_is_not_a_fault(self):
        # Review control R1-01: an explicit false marker suppresses the <synthetic> fallback, even
        # when the row quotes the overflow text.
        rows = [assistant_row(HAIKU, 3, cache_read=207_986),
                synthetic_row("A quote: Prompt is too long", marker=False)]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": True})
        self.assertFalse(pub["dead"])
        self.assertIsNone(pub["fault"])
        self.assertEqual(pub["used"], 207_989)
        self.assertEqual(pub["verdict"], "OK")
        self.assertNotIn("CONTEXT OVERFLOW", proc.stdout)
        self.assertNotIn("HOLD: do not retry", proc.stdout)

    def test_an_explicit_false_marker_clears_an_earlier_genuine_fault(self):
        # Review control R1-01, recovery: a genuine typed refusal, then an explicit non-error row.
        rows = [assistant_row(HAIKU, 3, cache_read=207_986),
                synthetic_row("Prompt is too long", marker=True),
                synthetic_row("Explaining Prompt is too long", marker=False)]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": True})
        self.assertFalse(pub["dead"])
        self.assertIsNone(pub["fault"])
        self.assertEqual(pub["verdict"], "OK")
        self.assertNotIn("CONTEXT OVERFLOW", proc.stdout)

    def test_a_true_marker_on_a_synthetic_row_is_still_a_fault_and_a_hold(self):
        rows = [assistant_row(HAIKU, 3, cache_read=899_000),
                synthetic_row("Prompt is too long", marker=True)]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": True})
        self.assertTrue(pub["dead"])
        self.assertEqual(pub["fault"], "native-overflow")
        self.assertIn("CONTEXT OVERFLOW", proc.stdout)

    def test_an_invalid_marker_value_is_an_ordinary_row_not_a_refusal(self):
        # Invalid behaves like explicit false: it suppresses the fallback. Only true is a refusal.
        for marker in ("true", 1, 0, None, []):
            with self.subTest(marker=marker):
                self.assertFalse(usage_of([synthetic_row("Prompt is too long", marker=marker)])[2])

    def test_the_synthetic_fallback_still_applies_when_the_marker_key_is_absent(self):
        # Transcripts that predate the marker key keep the legacy recognition.
        self.assertTrue(usage_of([synthetic_row("Prompt is too long")])[2])


class NativeAccounting(unittest.TestCase):
    """D. Claude usage is additive, read from each request's own usage block."""

    def test_first_cache_write_with_zero_reads_is_counted(self):
        # Before R2 the cache write was dropped: input 3 <= reads 0 fell to the else branch.
        used, observed_max, _, _ = usage_of([assistant_row(HAIKU, 3, cache_read=0, cache_write=20_000)])
        self.assertEqual(used, 20_003)
        self.assertEqual(observed_max, 20_003)

    def test_fresh_input_larger_than_reads_is_counted(self):
        # Before R2 the cache read was dropped: input 50,000 > reads 10,000 fell to else.
        used, _, _, _ = usage_of([assistant_row(HAIKU, 50_000, cache_read=10_000)])
        self.assertEqual(used, 60_000)

    def test_all_three_fields_add_up(self):
        used, _, _, _ = usage_of([assistant_row(HAIKU, 1_000, cache_read=20_000, cache_write=5_000)])
        self.assertEqual(used, 26_000)

    def test_latest_usage_drops_after_compaction(self):
        used, observed_max, _, _ = usage_of([assistant_row(HAIKU, 3, cache_read=899_000),
                                             assistant_row(HAIKU, 3, cache_read=119_997)])
        self.assertEqual(used, 120_000, "the last request's usage, not the pre-compaction peak")
        self.assertEqual(observed_max, 899_003)

    def test_a_row_without_usage_does_not_zero_the_reading(self):
        used, _, _, _ = usage_of([assistant_row(HAIKU, 3, cache_read=899_000),
                                  {"type": "assistant", "message": {"model": HAIKU,
                                   "content": [{"type": "text", "text": "x"}]}}])
        self.assertEqual(used, 899_003)


class PolicyOverride(unittest.TestCase):
    """C. Machine and human text agree; the raw band is kept beside an advisory verdict."""

    def test_configured_on_land_publishes_no_relay_verdict(self):
        rows = [assistant_row(HAIKU, 3, cache_read=719_000)]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": True})
        self.assertIn(OVERRIDE_MARK, proc.stdout)
        self.assertIn("relay-at-70%", proc.stdout)
        self.assertEqual(pub["pressure_band"], "LAND")
        self.assertNotIn(pub["verdict"], ("LAND", "CRITICAL"))
        self.assertFalse(pub["relay_recommended"])

    def test_unknown_policy_at_land_is_advisory_and_says_the_nudge_is_not_applied(self):
        rows = [assistant_row(HAIKU, 3, cache_read=719_000)]
        proc, pub = run_hook(rows, settings={})
        self.assertIn("not applied here", proc.stdout)
        self.assertEqual(pub["pressure_band"], "LAND")
        self.assertEqual(pub["verdict"], "NOTICE")
        self.assertEqual(pub["action"], "advisory")
        self.assertFalse(pub["relay_recommended"])

    def test_configured_off_still_relays_at_land(self):
        rows = [assistant_row(HAIKU, 3, cache_read=719_000)]
        _, pub = run_hook(rows, settings={"autoCompactEnabled": False})
        self.assertEqual(pub["verdict"], "LAND")
        self.assertEqual(pub["action"], "relay")
        self.assertTrue(pub["relay_recommended"])

    def test_env_disabled_still_relays_at_critical(self):
        rows = [assistant_row(HAIKU, 3, cache_read=899_000)]
        _, pub = run_hook(rows, settings={"autoCompactEnabled": True},
                          env={"DISABLE_COMPACT": "true"})
        self.assertEqual(pub["verdict"], "CRITICAL")
        self.assertTrue(pub["relay_recommended"])

    def test_unknown_capacity_never_relays_even_at_high_usage_with_compaction_off(self):
        rows = [assistant_row("claude-haiku-6", 3, cache_read=899_000)]
        for settings in ({"autoCompactEnabled": False}, {"autoCompactEnabled": True}, {}):
            with self.subTest(settings=settings):
                proc, pub = run_hook(rows, settings=settings)
                self.assertIn("WINDOW UNVERIFIED", proc.stdout)
                self.assertIn("LOWER BOUND", proc.stdout)
                for forbidden in ("LAND THIS SESSION", "THE WALL IS CLOSE", "Take no new work"):
                    self.assertNotIn(forbidden, proc.stdout)
                self.assertEqual(pub["verdict"], "UNKNOWN")
                self.assertEqual(pub["action"], "unverified")
                self.assertFalse(pub["relay_recommended"])
                self.assertIsNone(pub["pct"])
                self.assertIsNone(pub["window"])
                self.assertIsNone(pub["pressure_band"])


class SelectorBinding(unittest.TestCase):
    """B, at hook level. A configured `[1m]` selector cannot establish or refute a window. An unknown
    session stays UNKNOWN, with no percentage, no band and no relay. The one 200k assumption a
    selector can touch becomes UNVERIFIED rather than a wall."""

    def _assert_unverified(self, proc, pub):
        self.assertIsNone(pub["window"])
        self.assertIsNone(pub["pct"])
        self.assertTrue(pub["window_assumed"])
        self.assertEqual(pub["verdict"], "UNKNOWN")
        self.assertEqual(pub["action"], "unverified")
        self.assertFalse(pub["relay_recommended"])
        self.assertIn("WINDOW UNVERIFIED", proc.stdout)
        self.assertIn("cannot be bound to this session", proc.stdout)

    def test_a_stale_settings_selector_on_unknown_haiku_6_at_900k_is_unverified(self):
        # Review control R1-02: the previous code published 1M, 90%, CRITICAL and a relay here.
        rows = [assistant_row("claude-haiku-6", 3, cache_read=899_997)]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": False,
                                             "model": "claude-opus-4-1[1m]"})
        self._assert_unverified(proc, pub)

    def test_a_stale_environment_selector_on_unknown_sonnet_6_at_900k_is_unverified(self):
        rows = [assistant_row("claude-sonnet-6", 3, cache_read=899_997)]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": False},
                             env={"ANTHROPIC_MODEL": "claude-opus-4-1[1m]"})
        self._assert_unverified(proc, pub)

    def test_a_same_model_selector_on_opus_4_at_190k_is_unverified_not_a_200k_wall(self):
        rows = [assistant_row("claude-opus-4-1", 3, cache_read=189_997)]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": False,
                                             "model": "claude-opus-4-1[1m]"})
        self._assert_unverified(proc, pub)

    def test_without_the_selector_opus_4_at_190k_is_the_measured_200k_control(self):
        # The control for the test above: the 200k table itself is unchanged.
        rows = [assistant_row("claude-opus-4-1", 3, cache_read=189_997)]
        proc, pub = run_hook(rows, settings={"autoCompactEnabled": False})
        self.assertEqual(pub["window"], 200_000)
        self.assertFalse(pub["window_assumed"])
        self.assertEqual(pub["verdict"], "CRITICAL")
        self.assertTrue(pub["relay_recommended"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
