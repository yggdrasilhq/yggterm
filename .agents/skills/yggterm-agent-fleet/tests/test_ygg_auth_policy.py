"""Offline vault policy and fallback contracts; never read native auth."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import time
import types
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / 'ygg-auth.py'


class AuthPolicyTests(unittest.TestCase):
    def setUp(self):
        scratch = Path(os.environ.get('YGG_AUTH_TEST_SCRATCH', Path.home() / '.yggterm/scratchpad/auth-tests'))
        scratch.mkdir(parents=True, exist_ok=True)
        self.tmp = tempfile.TemporaryDirectory(dir=scratch)
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        spec = importlib.util.spec_from_file_location('auth_fixture', SCRIPT)
        self.m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.m)
        m = self.m
        m.STORE_ROOT = str(self.root / 'vault')
        m.LOCK_FILE = str(self.root / 'vault/.lock')
        m.ASSIGNMENTS_FILE = str(self.root / 'vault/.assignments.json')
        m.HOSTNAME = 'test-host'
        m.HARNESSES = {k: dict(v, auth_file=lambda h: str(self.root / (h + '.json'))) for k, v in m.HARNESSES.items()}
        self.addCleanup(patch.stopall)
        patch.object(m, 'usage_snapshot', side_effect=AssertionError('network forbidden')).start()
        patch.object(m, 'refresh_grant', side_effect=AssertionError('refresh forbidden')).start()

    def profile(self, slug, expired=False, live=False, harness='codex'):
        m = self.m
        record = {'tokens': {'access_token': 'fixture-' + slug, 'account_id': slug},
                  'expires_at': time.time() + (-100 if expired else 10000)}
        m.atomic_write_json(str(Path(m.store_dir(harness)) / (slug + '.json')), record)
        if live:
            m.atomic_write_json(m.HARNESSES[harness]['auth_file'](harness), record)
        return record

    def policy(self, **kw):
        p = dict(version=1, mode='local-pool', host='test-host', profiles=['a', 'b'],
                 source_authority='owner:test-fixture', automatic_fallback=True)
        p.update(kw)
        self.m.atomic_write_json(str(Path(self.m.store_dir('codex')) / '.policy.json'), p)

    def args(self, **kw):
        return types.SimpleNamespace(harness='codex', fast=True, cooldown_minutes=30, **kw)

    def test_headroom_minimum_uses_more_free_capacity(self):
        m = self.m
        for slug in ('a', 'b'):
            self.profile(slug)
        m.usage_snapshot = lambda rec: {'ok': True, 'primary': {'used_percent': 10 if rec['tokens']['account_id'] == 'a' else 80}}
        a, b = m.stored_profiles('codex')
        self.assertLess(m.rotate_score(a), m.rotate_score(b))
        m.usage_snapshot = lambda rec: {'ok': True, 'primary': {'used_percent': 10}, 'secondary': {'used_percent': 95 if rec['tokens']['account_id'] == 'a' else 20}}
        self.assertLess(m.rotate_score(b), m.rotate_score(a))
        m.usage_snapshot = lambda rec: {'ok': True}
        self.assertEqual(m.rotate_score(a)[0], 2)  # unknown, never full or free

    def test_policy_filtered_is_not_no_profiles(self):
        self.profile('a')
        self.m.atomic_write_json(self.m.ASSIGNMENTS_FILE, {'a': {'host': 'other-host'}})
        with self.assertRaises(self.m.AuthError) as ctx:
            self.m.pick_rotate_target('codex', measure=False)
        self.assertEqual(ctx.exception.details['counts']['policy_filtered'], 1)
        self.policy()
        self.assertEqual(self.m.pick_rotate_target('codex', measure=False)[0]['slug'], 'a')

    def test_invalid_policy_fails_closed(self):
        self.profile('a')
        for override in ({'version': 2}, {'host': 'other-host'}, {'source_authority': ''}, {'automatic_fallback': 'yes'}, {'unexpected': True}):
            self.policy(**override)
            with self.assertRaises(self.m.AuthError):
                self.m.pick_rotate_target('codex', measure=False)

    def test_invalid_and_expired_excluded_and_reason_retained(self):
        self.profile('a', expired=True)
        self.profile('b')
        self.m.record_failure('codex', 'b', 'refresh_invalid')
        with self.assertRaises(self.m.AuthError) as ctx:
            self.m.pick_rotate_target('codex', measure=False)
        self.assertEqual(ctx.exception.details['counts']['expired'], 1)
        self.assertEqual(ctx.exception.details['counts']['reauthorization_required'], 1)
        st = self.m.cmd_status(self.args())
        self.assertEqual(st['profiles'][1]['auth_health']['reason'], 'refresh_invalid')
        self.assertEqual(st['native_health'], 'unknown')

    def test_fallback_bad_current_healthy_local_preserves_grant_and_is_idempotent(self):
        old = self.profile('a', live=True)
        self.profile('b')
        self.policy()
        args = self.args(error_code='auth_invalid', event_id='fixture-event', expected_active_hash=self.m.record_hash(old))
        out = self.m.cmd_fallback(args)
        self.assertEqual(out['switched_to'], 'b')
        self.assertEqual(out['actual_effect'], 'unknown')
        self.assertEqual(out['session_action'], 'resume_or_restart_required')
        self.assertFalse(out['replay_authorized'])
        self.assertEqual(self.m.read_json(str(Path(self.m.store_dir('codex')) / 'a.json')), old)
        self.assertEqual(self.m.cmd_fallback(args), out)
        with self.assertRaises(self.m.AuthError):
            self.m.pick_rotate_target('codex', measure=False)

    def test_fallback_policy_and_typed_error_guards(self):
        old = self.profile('a', live=True)
        self.profile('b')
        args = self.args(error_code='auth_invalid', event_id='fixture-event', expected_active_hash=self.m.record_hash(old))
        with self.assertRaises(self.m.AuthError):
            self.m.cmd_fallback(args)
        self.policy()
        args.error_code = 'something went wrong'
        with self.assertRaises(self.m.AuthError):
            self.m.cmd_fallback(args)
        self.assertEqual(self.m.live_record('codex'), old)

    def test_plan_rotation_without_generic_oauth(self):
        m = self.m
        for slug in ('a', 'b'):
            m.atomic_write_json(str(Path(m.store_dir('zcode')) / (slug + '.json')), {'config': {'providerConfigRules': {'providerRules': [{'config': {'access': {'type': 'api-key', 'apiKey': 'fixture-' + slug}}}]}}})
        args = self.args()
        args.harness = 'zcode'
        self.assertEqual(m.cmd_rotate(args)['switched_to'], 'a')
        with self.assertRaises(m.AuthError):
            m.require_token_harness('zcode', 'refresh')
        with self.assertRaises(m.AuthError):
            m.require_token_harness('claude', 'refresh')
        self.assertTrue(m.capabilities('zcode')['rotate'])
        self.assertFalse(m.capabilities('zcode')['oauth_refresh'])

    def test_cooldown_never_reused(self):
        self.profile('a')
        self.m.mark_cooldown('codex', 'a', 30)
        with self.assertRaises(self.m.AuthError) as ctx:
            self.m.pick_rotate_target('codex', measure=False)
        self.assertEqual(ctx.exception.details['counts']['cooling'], 1)

    def test_refresh_rejection_retains_reason_without_body_and_preserves_record(self):
        import io
        import urllib.error
        m = self.m
        record = self.profile('a')
        # Test transport redaction independently of the normal no-network stub.
        spec = importlib.util.spec_from_file_location('auth_refresh_fixture', SCRIPT)
        raw = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(raw)
        record['tokens']['refresh_token'] = 'fixture-secret-refresh'
        m.atomic_write_json(str(Path(m.store_dir('codex')) / 'a.json'), record)
        rejection = urllib.error.HTTPError('https://example.test', 401, 'denied', {}, io.BytesIO(b'fixture-secret-refresh'))
        with patch.object(raw.urllib.request, 'urlopen', side_effect=rejection):
            with self.assertRaises(raw.AuthError) as ctx:
                raw.refresh_grant(record)
        self.assertNotIn('fixture-secret-refresh', str(ctx.exception))
        self.assertEqual(ctx.exception.details['reason'], 'refresh_invalid')
        m.refresh_grant = lambda rec: (_ for _ in ()).throw(m.AuthError(5, 'rejected', {'reason': 'refresh_invalid'}))
        with self.assertRaises(m.AuthError):
            m.cmd_refresh(types.SimpleNamespace(harness='codex', slug='a', force=False))
        self.assertEqual(m.read_json(str(Path(m.store_dir('codex')) / 'a.json')), record)
        self.assertEqual(m.profile_health('codex', 'a', record)['state'], 'reauthorization_required')

    def test_measured_invalid_candidate_excluded_and_good_fallback_wins(self):
        m = self.m
        self.profile('a')
        self.profile('b')
        m.usage_snapshot = lambda rec: ({'ok': False, 'http_status': 401} if rec['tokens']['account_id'] == 'a'
                                       else {'ok': True, 'primary': {'used_percent': 80}})
        with m.fleet_lock():
            self.assertEqual(m.pick_rotate_target('codex')[0]['slug'], 'b')
        self.assertEqual(m.cmd_status(self.args())['profiles'][0]['auth_health']['reason'], 'auth_invalid')

    def test_measured_exhausted_not_selected_but_unknown_is_not_free(self):
        m = self.m
        self.profile('a')
        m.usage_snapshot = lambda rec: {'ok': True, 'limit_reached': True, 'primary': {'used_percent': 100}}
        with self.assertRaises(m.AuthError):
            m.pick_rotate_target('codex')
        m.usage_snapshot = lambda rec: {'ok': False, 'error': 'unavailable'}
        self.assertEqual(m.pick_rotate_target('codex')[0]['slug'], 'a')
        self.assertEqual(m.cmd_status(self.args())['profiles'][0]['quota']['state'], 'unknown')

    def test_rate_fallback_preserves_longer_cooldown_and_history(self):
        m = self.m
        old = self.profile('a', live=True)
        self.profile('b')
        self.policy()
        until = m.mark_cooldown('codex', 'a', 120)
        m.record_failure('codex', 'a', 'token_expired')
        args = self.args(error_code='rate_limited', event_id='rate-event', expected_active_hash=m.record_hash(old))
        out = m.cmd_fallback(args)
        self.assertEqual(out['switched_to'], 'b')
        self.assertEqual(m.cooling_until('codex', 'a'), until)
        self.assertEqual(m.profile_health('codex', 'a', old)['history'][0]['reason'], 'token_expired')

    def test_failed_fallback_event_cannot_repeat_or_consume_reset(self):
        m = self.m
        old = self.profile('a', live=True)
        self.profile('b', expired=True)
        self.policy()
        args = self.args(error_code='auth_invalid', event_id='failed-event', expected_active_hash=m.record_hash(old))
        with self.assertRaises(m.AuthError) as ctx:
            m.cmd_fallback(args)
        self.assertEqual(ctx.exception.details['counts']['expired'], 1)
        self.assertEqual(m.live_record('codex'), old)
        with self.assertRaises(m.AuthError) as ctx:
            m.cmd_fallback(args)
        self.assertEqual(ctx.exception.details['reason'], 'event_incomplete')

    def test_stale_consumer_binding_and_event_collision_are_rejected(self):
        m = self.m
        old = self.profile('a', live=True)
        self.profile('b')
        self.policy()
        args = self.args(error_code='auth_invalid', event_id='collision-event', expected_active_hash='stale')
        with self.assertRaises(m.AuthError):
            m.cmd_fallback(args)
        self.assertEqual(m.live_record('codex'), old)
        args.expected_active_hash = m.record_hash(old)
        m.cmd_fallback(args)
        args.error_code = 'rate_limited'
        with self.assertRaises(m.AuthError):
            m.cmd_fallback(args)

    def test_named_profile_and_opaque_capture_on_leave_keep_grant(self):
        m = self.m
        a = self.profile('named-a', live=True)
        self.profile('b')
        out = m.cmd_rotate(self.args())
        self.assertEqual(out['captured_before_leaving']['slug'], 'named-a')
        self.assertEqual(m.read_json(str(Path(m.store_dir('codex')) / 'named-a.json')), a)
        # An opaque externally updated record is archived before replacement.
        opaque = {'claudeAiOauth': {'accessToken': 'fixture-new', 'refreshToken': 'fixture-refresh'}}
        m.atomic_write_json(m.HARNESSES['claude']['auth_file']('claude'), opaque)
        m.atomic_write_json(str(Path(m.store_dir('claude')) / 'b.json'), {'claudeAiOauth': {'accessToken': 'fixture-other'}})
        with m.fleet_lock():
            out = m.switch('claude', 'b', lock_held=True)
        self.assertEqual(m.read_json(out['captured_before_leaving']['path']), opaque)

    def test_cli_harness_positions_inventory_and_machine_error(self):
        import subprocess
        import sys
        env = dict(os.environ, HOME=str(self.root), CODEX_HOME=str(self.root / 'codex'),
                   CLAUDE_CONFIG_DIR=str(self.root / 'claude'), YGG_AUTH_HOME=self.m.STORE_ROOT,
                   ZCODE_AUTH_FILE=str(self.root / 'zcode.json'), AUTOCLAW_PLAN_FILE=str(self.root / 'autoclaw.json'))
        for argv in (['--harness', 'claude', 'status'], ['status', '--harness', 'claude'], ['status', '--harness', 'all']):
            p = subprocess.run([sys.executable, str(SCRIPT), *argv, '--json'], env=env, capture_output=True, text=True)
            self.assertEqual(p.returncode, 0, p.stderr)
            self.assertEqual(json.loads(p.stdout)['schema_version'], 1)
        p = subprocess.run([sys.executable, str(SCRIPT), 'rotate', '--harness', 'codex', '--fast', '--json'], env=env, capture_output=True, text=True)
        self.assertEqual(p.returncode, 2)
        out = json.loads(p.stdout)
        self.assertEqual(out['error']['counts']['stored'], 0)
        self.assertEqual(out['outcome'], 'error')

    def test_unreadable_and_invalid_records_reported_without_activation(self):
        m = self.m
        directory = Path(m.store_dir('codex'))
        m.atomic_write_json(str(directory / 'empty.json'), {})
        (directory / 'broken.json').write_text('{')
        with self.assertRaises(m.AuthError) as ctx:
            m.pick_rotate_target('codex', measure=False)
        counts = ctx.exception.details['counts']
        self.assertEqual((counts['invalid_record'], counts['unreadable']), (1, 1))
        self.assertEqual(len(m.cmd_status(self.args())['profiles']), 2)

    def test_auto_fallback_all_filtered_is_explicit_and_keeps_active(self):
        m = self.m
        old = self.profile('a', live=True)
        self.profile('b')
        self.policy(profiles=['a'])
        args = self.args(error_code='auth_invalid', event_id='filtered-event', expected_active_hash=m.record_hash(old))
        with self.assertRaises(m.AuthError) as ctx:
            m.cmd_fallback(args)
        self.assertEqual(ctx.exception.details['counts']['policy_filtered'], 1)
        self.assertEqual(m.live_record('codex'), old)

    def test_native_change_during_fallback_cannot_replace_new_grant(self):
        m = self.m
        old = self.profile('a', live=True)
        self.profile('b')
        self.policy()
        choose = m.pick_rotate_target
        changed = dict(old, last_refresh='fixture-new-refresh')
        def concurrent_change(*args, **kwargs):
            target = choose(*args, **kwargs)
            m.atomic_write_json(m.HARNESSES['codex']['auth_file']('codex'), changed)
            return target
        m.pick_rotate_target = concurrent_change
        args = self.args(error_code='auth_invalid', event_id='race-event', expected_active_hash=m.record_hash(old))
        with self.assertRaises(m.AuthError):
            m.cmd_fallback(args)
        self.assertEqual(m.live_record('codex'), changed)

    def test_plan_adapters_never_measure_codex_quota(self):
        m = self.m
        m.atomic_write_json(str(Path(m.store_dir('autoclaw')) / 'a.json'), {'auth': {'accessToken': 'fixture-access'}})
        args = self.args()
        args.harness = 'autoclaw'
        args.fast = False
        self.assertEqual(m.cmd_rotate(args)['switched_to'], 'a')


if __name__ == '__main__':
    unittest.main()
