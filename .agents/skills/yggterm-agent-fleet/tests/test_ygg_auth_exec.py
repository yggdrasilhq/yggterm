"""Offline exec/preflight consumer contracts; never read native auth, never network.

Covers the pre-launch consumer surface: preflight receipt binding, proven
grant gate, one bounded refresh, one typed-failure fallback with CAS dedup,
concurrent-change refusal before activation, argv/exit passthrough and secret
hygiene. All auth paths are isolated fixtures; `--preflight-only` is a pure
read, and spawn is asserted through the _exec_spawn seam or real child exits.
"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import types
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / 'ygg-auth.py'
SECRET = 'fixture-secret-token-do-not-print'


class AuthExecTests(unittest.TestCase):
    def setUp(self):
        scratch = Path(os.environ.get('YGG_AUTH_TEST_SCRATCH', Path.home() / '.yggterm/scratchpad/auth-tests'))
        scratch.mkdir(parents=True, exist_ok=True)
        self.tmp = tempfile.TemporaryDirectory(dir=scratch)
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        spec = importlib.util.spec_from_file_location('auth_exec_fixture', SCRIPT)
        self.m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.m)
        m = self.m
        m.STORE_ROOT = str(self.root / 'vault')
        m.LOCK_FILE = str(self.root / 'vault/.lock')
        m.ASSIGNMENTS_FILE = str(self.root / 'vault/.assignments.json')
        m.HOSTNAME = 'test-host'
        m.HARNESSES = {k: dict(v, auth_file=lambda h: str(self.root / (h + '.json'))) for k, v in m.HARNESSES.items()}
        self.spawns = []
        self.spawn_receipts = []
        patch.object(m, '_exec_spawn',
                     side_effect=lambda argv, receipt=None: (self.spawns.append(argv),
                                                             self.spawn_receipts.append(receipt))).start()
        patch.object(m, 'usage_snapshot', side_effect=AssertionError('network forbidden')).start()
        patch.object(m, 'refresh_grant', side_effect=AssertionError('refresh forbidden')).start()
        self.addCleanup(patch.stopall)

    def cli_live(self, record, harness='codex'):
        """Write the live auth file where the REAL script resolves it
        (CODEX_HOME/auth.json), not the module-fixture override path."""
        path = Path(self.root) / harness / 'auth.json'
        path.parent.mkdir(parents=True, exist_ok=True)
        self.m.atomic_write_json(str(path), record)
        return record

    def profile(self, slug, expired=False, live=False, harness='codex', token=SECRET):
        m = self.m
        record = {'tokens': {'access_token': token, 'refresh_token': 'fixture-refresh-' + slug,
                             'account_id': slug},
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
        base = dict(harness='codex', preflight_only=False, measure=False,
                    typed_failure=None, event_id=None, observed_active_hash=None,
                    native_argv=['fixture-native'], json=False)
        base.update(kw)
        return types.SimpleNamespace(**base)

    def cli(self, *argv, extra_env=None):
        env = dict(os.environ, HOME=str(self.root), CODEX_HOME=str(self.root / 'codex'),
                   CLAUDE_CONFIG_DIR=str(self.root / 'claude'), YGG_AUTH_HOME=self.m.STORE_ROOT,
                   ZCODE_AUTH_FILE=str(self.root / 'zcode.json'), AUTOCLAW_PLAN_FILE=str(self.root / 'autoclaw.json'))
        env.update(extra_env or {})
        return subprocess.run([sys.executable, str(SCRIPT), *argv], env=env,
                              capture_output=True, text=True, timeout=60)

    def test_preflight_receipt_binds_hash_provider_harness_and_reads_only(self):
        m = self.m
        record = self.profile('a', live=True)
        live_path = m.HARNESSES['codex']['auth_file']('codex')
        before = os.stat(live_path)
        out = m.cmd_exec(self.args(preflight_only=True))
        self.assertEqual(out['outcome'], 'ready')
        self.assertEqual(out['harness'], 'codex')
        self.assertEqual(out['provider'], 'openai')
        self.assertEqual(out['active_binding'], 'a')
        self.assertTrue(out['grant_binding']['proven'])
        self.assertEqual(out['grant_binding']['record_hash'], m.record_hash(record))
        self.assertEqual(out['native_health'], 'unknown')
        self.assertEqual(out['actual_effect'], 'unknown')
        self.assertFalse(out['native_exec']['spawned'])
        self.assertFalse(out['meter']['measured'])
        after = os.stat(live_path)
        self.assertEqual((before.st_mtime_ns, before.st_size), (after.st_mtime_ns, after.st_size))
        self.assertEqual(m.stored_profiles('codex')[0]['path'], str(Path(m.store_dir('codex')) / 'a.json'))

    def test_unproven_grant_blocks_spawn(self):
        m = self.m
        m.atomic_write_json(m.HARNESSES['codex']['auth_file']('codex'),
                            {'tokens': {'access_token': SECRET, 'account_id': 'x'}, 'expires_at': time.time() + 1000})
        with self.assertRaises(m.AuthError) as ctx:
            m.cmd_exec(self.args(preflight_only=True))
        self.assertEqual(ctx.exception.details['reason'], 'grant_unproven')
        self.assertFalse(self.spawns)

    def test_health_failure_blocks_spawn_until_reauth(self):
        m = self.m
        record = self.profile('a', live=True)
        m.record_failure('codex', 'a', 'auth_invalid')
        with self.assertRaises(m.AuthError) as ctx:
            m.cmd_exec(self.args(preflight_only=True))
        self.assertEqual(ctx.exception.details['reason'], 'auth_health_failed')
        self.assertEqual(m.live_record('codex'), record)
        self.assertFalse(self.spawns)

    def test_expired_refreshable_takes_exactly_one_canonical_refresh(self):
        m = self.m
        old = self.profile('a', expired=True, live=True)
        calls = []

        def fixture_refresh(record):
            calls.append(record)
            fresh = json.loads(json.dumps(record))
            fresh['tokens']['access_token'] = 'fixture-refreshed-access'
            fresh['tokens']['refresh_token'] = 'fixture-rotated-refresh'
            fresh['expires_at'] = time.time() + 10000
            return fresh

        m.refresh_grant = fixture_refresh
        m.cmd_exec(self.args())
        receipt = self.spawn_receipts[-1]
        self.assertEqual(len(calls), 1)
        self.assertEqual(receipt['activation'], {'performed': True, 'kind': 'refresh_once', 'fallback': False})
        live = m.live_record('codex')
        self.assertEqual(live['tokens']['access_token'], 'fixture-refreshed-access')
        self.assertEqual(live['tokens']['refresh_token'], 'fixture-rotated-refresh')
        self.assertEqual(m.read_json(str(Path(m.store_dir('codex')) / 'a.json')), live)
        self.assertNotEqual(live, old)
        self.assertEqual(self.spawns, [['fixture-native']])

    def test_invalid_refresh_records_failure_and_blocks(self):
        m = self.m
        old = self.profile('a', expired=True, live=True)

        def rejected(record):
            raise m.AuthError(5, 'rejected', {'reason': 'refresh_invalid'})

        m.refresh_grant = rejected
        with self.assertRaises(m.AuthError) as ctx:
            m.cmd_exec(self.args())
        self.assertEqual(ctx.exception.details.get('reason'), 'refresh_invalid')
        self.assertEqual(m.live_record('codex'), old)
        self.assertEqual(m.profile_health('codex', 'a', old)['state'], 'reauthorization_required')
        self.assertFalse(self.spawns)

    def test_foreign_writer_is_held_without_refresh_or_copy(self):
        m = self.m
        old = self.profile('a', expired=True, live=True)
        m.stamp_provenance('codex', 'a', writer='other-host')
        with self.assertRaises(m.AuthError) as ctx:
            m.cmd_exec(self.args(preflight_only=True))
        self.assertEqual(ctx.exception.details['reason'], 'foreign_writer_held')
        self.assertEqual(m.live_record('codex'), old)
        self.assertFalse(self.spawns)

    def test_expired_without_refresher_blocks_without_codex_calls(self):
        m = self.m
        record = {'claudeAiOauth': {'accessToken': SECRET, 'refreshToken': 'fixture-r',
                                    'expiresAt': time.time() * 1000 - 1000}}
        m.atomic_write_json(str(Path(m.store_dir('claude')) / 'a.json'), record)
        m.atomic_write_json(m.HARNESSES['claude']['auth_file']('claude'), record)
        with self.assertRaises(m.AuthError) as ctx:
            m.cmd_exec(self.args(harness='claude', native_argv=['fixture-native']))
        self.assertEqual(ctx.exception.details['reason'], 'reauthorization_required')
        self.assertFalse(ctx.exception.details['refreshable'])
        self.assertFalse(self.spawns)

    def test_concurrent_native_change_before_activation_refuses(self):
        m = self.m
        old = self.profile('a', live=True)
        self.profile('b')
        changed = dict(old, last_refresh='fixture-concurrent')
        real_meter = m.meter_snapshot

        def racing_meter(harness, record, measure=False):
            m.atomic_write_json(m.HARNESSES['codex']['auth_file']('codex'), changed)
            return real_meter(harness, record, measure=False)

        m.meter_snapshot = racing_meter
        with self.assertRaises(m.AuthError) as ctx:
            m.cmd_exec(self.args())
        self.assertEqual(ctx.exception.details['reason'], 'concurrent_change')
        self.assertEqual(m.live_record('codex'), changed)
        self.assertFalse(self.spawns)

    def test_typed_failure_fallback_once_then_spawn_healthy_target(self):
        m = self.m
        old = self.profile('a', live=True)
        self.profile('b')
        self.policy()
        m.cmd_exec(self.args(typed_failure='rate_limited', event_id='e1',
                             observed_active_hash=m.record_hash(old)))
        receipt = self.spawn_receipts[-1]
        self.assertEqual(m.live_record('codex')['tokens']['account_id'], 'b')
        self.assertEqual(receipt['fallback_event_id'], 'e1')
        self.assertEqual(receipt['fallback_switched_to'], 'b')
        self.assertTrue(receipt['activation']['fallback'])
        self.assertIsNotNone(m.cooling_until('codex', 'a'))
        self.assertEqual(m.profile_health('codex', 'a', old)['state'], 'unknown')
        self.assertEqual(self.spawns, [['fixture-native']])

    def test_typed_failure_event_dedup_and_collision(self):
        m = self.m
        old = self.profile('a', live=True)
        self.profile('b')
        self.policy()
        m.cmd_exec(self.args(typed_failure='auth_invalid', event_id='e2',
                             observed_active_hash=m.record_hash(old)))
        self.assertEqual(self.spawns, [['fixture-native']])
        m.cmd_exec(self.args(typed_failure='auth_invalid', event_id='e2',
                             observed_active_hash=m.record_hash(old)))
        first, second = self.spawn_receipts[0], self.spawn_receipts[-1]
        self.assertEqual(second['fallback_switched_to'], first['fallback_switched_to'])
        self.assertEqual(self.spawns, [['fixture-native'], ['fixture-native']])
        with self.assertRaises(m.AuthError) as ctx:
            m.cmd_exec(self.args(typed_failure='auth_invalid', event_id='e2',
                                 observed_active_hash='stale-hash'))
        self.assertEqual(ctx.exception.code, 6)

    def test_malformed_policy_fails_closed_before_spawn(self):
        m = self.m
        old = self.profile('a', live=True)
        self.profile('b')
        self.policy(version=2)
        with self.assertRaises(m.AuthError):
            m.cmd_exec(self.args(typed_failure='auth_invalid', event_id='e3',
                                 observed_active_hash=m.record_hash(old)))
        self.assertEqual(m.live_record('codex'), old)
        self.assertFalse(self.spawns)

    def test_all_filtered_fallback_keeps_active_and_refuses(self):
        m = self.m
        old = self.profile('a', live=True)
        self.profile('b')
        self.policy(profiles=['a'])
        with self.assertRaises(m.AuthError) as ctx:
            m.cmd_exec(self.args(typed_failure='auth_invalid', event_id='e4',
                                 observed_active_hash=m.record_hash(old)))
        self.assertEqual(ctx.exception.details['counts']['policy_filtered'], 1)
        self.assertEqual(m.live_record('codex'), old)
        self.assertFalse(self.spawns)

    def test_rate_limited_and_auth_invalid_are_distinct_classes(self):
        m = self.m
        old = self.profile('a', live=True)
        self.profile('b')
        self.policy()
        m.cmd_exec(self.args(typed_failure='rate_limited', event_id='r1',
                             observed_active_hash=m.record_hash(old), native_argv=['n']))
        self.assertIsNotNone(m.cooling_until('codex', 'a'))
        self.assertIsNone(m.read_metadata(str(Path(m.health_path('codex')))).get('a'))
        old2 = self.profile('a', live=True)
        m.atomic_write_json(str(Path(m.store_dir('codex')) / '.cooldown.json'), {})
        m.cmd_exec(self.args(typed_failure='auth_invalid', event_id='r2',
                             observed_active_hash=m.record_hash(old2), native_argv=['n']))
        self.assertIsNone(m.cooling_until('codex', 'a'))
        self.assertEqual(m.profile_health('codex', 'a', old2)['state'], 'reauthorization_required')

    def test_meter_unknown_unsupported_or_measured_never_carries_secrets(self):
        m = self.m
        zcode_record = {'config': {'providerConfigRules': {'providerRules': [
            {'config': {'access': {'type': 'api-key', 'apiKey': SECRET}}}]}}}
        m.atomic_write_json(str(Path(m.store_dir('zcode')) / 'a.json'), zcode_record)
        m.atomic_write_json(m.HARNESSES['zcode']['auth_file']('zcode'), zcode_record)
        out = m.cmd_exec(self.args(harness='zcode', preflight_only=True, measure=True))
        self.assertEqual(out['provider'], 'zai')
        self.assertEqual(out['meter']['state'], 'unsupported')
        self.profile('a', live=True)
        m.usage_snapshot = lambda rec: {'ok': True, 'email': 'someone@example.test', 'plan_type': 'plus',
                                        'limit_reached': False, 'allowed': True,
                                        'primary': {'used_percent': 20, 'reset_in_minutes': 120},
                                        'secondary': None}
        out = m.cmd_exec(self.args(preflight_only=True, measure=True))
        self.assertEqual(out['meter']['measured'], True)
        self.assertEqual(out['meter']['primary']['used_percent'], 20)
        blob = json.dumps(out)
        self.assertNotIn('someone@example.test', blob)
        self.assertNotIn(SECRET, blob)

    def test_cli_preflight_receipt_exit_codes_and_secret_hygiene(self):
        self.cli_live(self.profile('a'))
        p = self.cli('exec', '--harness', 'codex', '--preflight-only', '--json')
        self.assertEqual(p.returncode, 0, p.stderr)
        receipt = json.loads(p.stdout)
        self.assertEqual(receipt['outcome'], 'ready')
        self.assertEqual(receipt['provider'], 'openai')
        self.assertNotIn(SECRET, p.stdout + p.stderr)
        p = self.cli('exec', '--harness', 'codex', '--', 'sh', '-c', 'exit 7')
        self.assertEqual(p.returncode, 7)
        self.assertNotIn(SECRET, p.stdout + p.stderr)
        # --measure on codex is the one optional network touch; the subprocess
        # cannot stub it, so the fixture routes it at a dead local proxy —
        # the meter must degrade to unknown, never fake healthy or leak.
        p = self.cli('exec', '--harness', 'codex', '--preflight-only', '--json', '--measure',
                     extra_env={'http_proxy': 'http://127.0.0.1:9', 'https_proxy': 'http://127.0.0.1:9'})
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertFalse(json.loads(p.stdout)['meter']['measured'])

    def test_cli_unproven_grant_and_missing_binary_exit_codes(self):
        self.cli_live({'tokens': {'access_token': SECRET, 'account_id': 'x'},
                       'expires_at': time.time() + 1000})
        p = self.cli('exec', '--harness', 'codex', '--preflight-only', '--json')
        self.assertEqual(p.returncode, 5)
        self.assertEqual(json.loads(p.stdout)['error']['reason'], 'grant_unproven')
        self.assertNotIn(SECRET, p.stdout + p.stderr)
        self.cli_live(self.profile('a'))
        p = self.cli('exec', '--harness', 'codex', '--', 'definitely-not-a-native-binary-xyz')
        self.assertEqual(p.returncode, 6)
        self.assertIn('could not start', p.stderr)
        p = self.cli('exec', '--harness', 'codex')
        self.assertEqual(p.returncode, 6)

    def test_cli_argv_spaces_and_special_characters_pass_verbatim(self):
        self.cli_live(self.profile('a'))
        out_file = self.root / 'argv-capture.json'
        child = ('import json,sys;open(sys.argv[1],"w").write('
                 'json.dumps(sys.argv[2:]))')
        expected = ['a b', 'c;d', '"quoted"', '$HOME', '--flag=x y']
        argv = [sys.executable, '-c', child, str(out_file)] + expected
        p = self.cli('exec', '--harness', 'codex', '--', *argv)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(json.loads(out_file.read_text()), expected)


if __name__ == '__main__':
    unittest.main()
