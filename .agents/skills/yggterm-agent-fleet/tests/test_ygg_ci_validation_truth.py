"""CI identity and validation must be observed, never inferred from text labels."""
import importlib.util
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_ygg_ci_readable_output import load_ci, new_home, make_repo_with_remote, seed_sub

SCRIPT = Path(os.environ.get('YGG_CI_TEST_SCRIPT', Path(__file__).resolve().parents[1] / 'ygg-ci.py'))


class TruthfulCI(unittest.TestCase):
    def setUp(self):
        scratch = (Path.home() / '.yggterm/scratchpad/ci-validation-truth').resolve()
        scratch.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=scratch)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.home = new_home(self.root, 'state')
        self.m = load_ci(SCRIPT, self.home)

    def test_only_actual_python_script_subcommand_is_a_watcher(self):
        m = self.m
        for argv in (['/usr/bin/python3', '/invented/ygg-ci.py', 'watch'],
                     ['python3.14', '-u', '/invented/ygg-ci.py', 'watch', '--interval', '300']):
            self.assertTrue(m._is_watcher_argv(argv))
        for argv in (['bash', '-c', 'python3 ygg-ci.py watch'],
                     ['python3', '-c', 'print("ygg-ci.py watch")'],
                     ['python3', 'other.py', 'ygg-ci.py', 'watch'],
                     ['python3', 'ygg-ci.py', 'status', '--note', 'watch']):
            self.assertFalse(m._is_watcher_argv(argv))

    def test_unreadable_named_owner_is_unknown_and_cannot_spawn_a_twin(self):
        m = self.m
        m.PIDFILE.write_text('777')
        with patch.object(m.os, 'kill'), patch.object(m, '_watcher_argv', side_effect=PermissionError):
            self.assertIsNone(m.watcher_alive())
            with patch.object(m.subprocess, 'Popen') as spawn:
                self.assertFalse(m._ensure_watcher_locked())
                spawn.assert_not_called()

    def test_first_subscription_starts_the_shared_global_owner(self):
        m = self.m
        config = {'repo': str(self.root), 'host': 'dev', 'remote': 'origin', 'interval': 86400}
        with patch.object(m, '_repo_for', return_value=(self.root, config)), \
                patch.object(m, 'this_host', return_value='dev'), \
                patch.object(m, '_run', return_value=subprocess.CompletedProcess([], 0, 'a' * 40, '')), \
                patch.object(m, 'ensure_watcher', return_value=True) as owner:
            args = SimpleNamespace(project='another-project', lane='lane/example', repo=None, want='next', by='fixture')
            self.assertEqual(m.cmd_subscribe(args), 0)
        owner.assert_called_once_with(interval=m.DEFAULT_INTERVAL, project=None)

    def test_project_scoped_owner_cannot_claim_global_supervision(self):
        m = self.m
        m.PIDFILE.write_text('777')
        argv = ['python3', '/invented/ygg-ci.py', 'watch', '--project', 'one-project']
        with patch.object(m.os, 'kill'), patch.object(m, '_watcher_argv', return_value=argv):
            self.assertTrue(m.watcher_alive())
            self.assertFalse(m._ensure_watcher_locked(project=None))
            self.assertTrue(m._ensure_watcher_locked(project='one-project'))
            self.assertFalse(m._ensure_watcher_locked(project='different-project'))

    def test_markdown_after_a_failed_build_really_executes_validation(self):
        m = self.m
        repo = make_repo_with_remote(self.root)
        g = lambda *args: subprocess.check_output(['git', '-C', str(repo), *args], text=True).strip()
        baseline = g('rev-parse', 'HEAD')
        g('checkout', '-b', 'lane/example/docs')
        (repo / 'README.md').write_text('invented changed documentation\n')
        g('add', 'README.md')
        g('commit', '-m', 'invented documentation follow-up')
        g('push', '-q', 'origin', 'lane/example/docs')
        g('checkout', 'main')
        marker = self.root / 'checks-ran'
        code = f'from pathlib import Path;import sys;Path({str(marker)!r}).write_text("ran");sys.exit(1)'
        config = {'repo': str(repo), 'host': 'dev', 'remote': 'origin', 'main_branch': 'main',
                  'build': shlex.join([sys.executable, '-c', code]), 'deploy': '', 'gates': '', 'push': False}
        seed_sub(m, self.home, 'lane/example/docs', repo)
        m.BUILDS.mkdir(parents=True, exist_ok=True)
        (m.BUILDS / 'yggterm--previous-failed.json').write_text(json.dumps({
            'id': 'previous', 'status': 'build-failed', 'build_ok': False, 'sha': baseline}))
        with patch.object(m, '_project_cfg', return_value=config), \
                patch.object(m, 'this_host', return_value='dev'), \
                patch.object(m, '_emit_event'), \
                patch.object(m, '_sccache_stats', return_value={}):
            result = m._do_tick_project('yggterm')
        self.assertTrue(marker.is_file(), 'configured checks must actually run')
        self.assertEqual(result['status'], 'build-failed')
        self.assertEqual(g('rev-parse', 'HEAD'), baseline)

    def test_verify_runs_current_checks_without_integration_or_deployment(self):
        m = self.m
        repo = make_repo_with_remote(self.root)
        baseline = subprocess.check_output(['git', '-C', str(repo), 'rev-parse', 'HEAD'], text=True).strip()
        marker = self.root / 'verified'
        code = f'from pathlib import Path;Path({str(marker)!r}).write_text("ran")'
        config = {'repo': str(repo), 'host': 'dev', 'main_branch': 'main',
                  'build': shlex.join([sys.executable, '-c', code]),
                  'gates': '', 'push': True, 'deploy': 'must never execute'}
        with patch.object(m, '_project_cfg', return_value=config), \
                patch.object(m, 'this_host', return_value='dev'), patch.object(m, '_emit_event'):
            result = m._with_project_lease('fixture', m._verify_project_locked)
        self.assertEqual(result['status'], 'verified')
        self.assertTrue(marker.is_file())
        self.assertFalse(result['pushed'])
        self.assertFalse(result['deploy_attempted'])
        self.assertTrue(Path(result['receipt']).is_file())
        self.assertEqual(subprocess.check_output(['git', '-C', str(repo), 'rev-parse', 'HEAD'], text=True).strip(), baseline)

    def test_verify_rejects_subject_drift_after_checks(self):
        m = self.m
        repo = make_repo_with_remote(self.root)
        code = 'from pathlib import Path;Path("README.md").write_text("invented build drift")'
        config = {'repo': str(repo), 'host': 'dev', 'main_branch': 'main',
                  'build': shlex.join([sys.executable, '-c', code]), 'gates': ''}
        with patch.object(m, '_project_cfg', return_value=config), \
                patch.object(m, 'this_host', return_value='dev'), patch.object(m, '_emit_event'):
            result = m._with_project_lease('fixture', m._verify_project_locked)
        self.assertEqual(result['status'], 'verification-failed')
        self.assertFalse(result['subject_stable'])

    def test_unpublished_local_main_requires_checks_and_retains_failure(self):
        m = self.m
        repo = make_repo_with_remote(self.root)
        g = lambda *a: subprocess.check_output(['git', '-C', str(repo), *a], text=True).strip()
        g('merge', '--no-ff', '-m', 'invented unpublished merge', 'lane/example/dry')
        unpublished = g('rev-parse', 'HEAD')
        marker = self.root / 'local-main-checked'
        code = f'from pathlib import Path;import sys;Path({str(marker)!r}).write_text("ran");sys.exit(1)'
        config = {'repo': str(repo), 'host': 'dev', 'remote': 'origin', 'main_branch': 'main',
                  'build': shlex.join([sys.executable, '-c', code]), 'deploy': '', 'gates': '', 'push': True}
        subscription = seed_sub(m, self.home, 'lane/example/dry', repo)
        with patch.object(m, '_project_cfg', return_value=config), \
                patch.object(m, 'this_host', return_value='dev'), \
                patch.object(m, '_emit_event'), patch.object(m, '_sccache_stats', return_value={}):
            result = m._do_tick_project('yggterm')
        self.assertEqual(result['status'], 'build-failed')
        self.assertTrue(marker.is_file())
        self.assertTrue(subscription.is_file(), 'failed checks cannot consume unpublished work')
        self.assertEqual(g('rev-parse', 'HEAD'), unpublished)
        self.assertNotEqual(g('rev-parse', 'origin/main'), unpublished)

    def test_receipt_directory_obstruction_is_typed_held_and_preserved(self):
        m = self.m
        repo = make_repo_with_remote(self.root)
        obstruction = m.CI_STATE / 'verifications'
        obstruction.write_text('invented owner file; preserve')
        config = {'repo': str(repo), 'host': 'dev', 'main_branch': 'main',
                  'build': shlex.join([sys.executable, '-c', 'pass']), 'gates': ''}
        with patch.object(m, '_project_cfg', return_value=config), \
                patch.object(m, 'this_host', return_value='dev'), patch.object(m, '_emit_event'):
            result = m._with_project_lease('fixture', m._verify_project_locked)
        self.assertEqual(result['status'], 'verification-held')
        self.assertEqual(result['receipt_publication'], 'not-created')
        self.assertEqual(obstruction.read_text(), 'invented owner file; preserve')


if __name__ == '__main__':
    unittest.main()
