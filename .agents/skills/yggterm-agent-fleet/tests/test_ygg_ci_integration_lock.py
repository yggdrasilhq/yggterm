"""Real process leases prevent simultaneous checkout mutation across CI labels."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(os.environ.get("YGG_CI_TEST_SCRIPT",
                            Path(__file__).resolve().parents[1] / "ygg-ci.py"))
CHILD = """
import importlib.util,json,sys
from pathlib import Path
sys.path.insert(0,str(Path(sys.argv[1]).parent))
s=importlib.util.spec_from_file_location('ci_fixture',sys.argv[1])
m=importlib.util.module_from_spec(s);s.loader.exec_module(m)
m._project_cfg=lambda project:{'repo':sys.argv[2]}
def held(project,dry=False,**kwargs):
 print('ENTERED',file=sys.stderr,flush=True)
 sys.stdin.read(1)
 if project=='raise':raise RuntimeError('invented build failure')
 return {'status':'clean'}
m._do_tick_project_locked=held
print(json.dumps(m._do_tick_project(sys.argv[3])),flush=True)
"""


class IntegrationLease(unittest.TestCase):
    def setUp(self):
        scratch = (Path.home() / '.yggterm/scratchpad/test-ci-integration-lease').resolve()
        scratch.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=scratch)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        self.git('init', '-q', '-b', 'main')
        self.git('config', 'user.name', 'Fixture')
        self.git('config', 'user.email', 'fixture@example.test')
        self.git('commit', '-q', '--allow-empty', '-m', 'invented fixture')
        self.env = dict(os.environ, YGGTERM_HOME=str(self.root / 'state'))

    def git(self, *args):
        subprocess.run(['git', '-C', str(self.repo), *args], check=True,
                       capture_output=True)

    def child(self, project='first', repo=None):
        return subprocess.Popen([sys.executable, '-c', CHILD, str(SCRIPT),
                                 str(repo or self.repo), project],
                                env=self.env, stdin=subprocess.PIPE,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                text=True)

    def finish(self, process):
        if process.poll() is None:
            process.communicate('x', timeout=10)

    def test_same_repository_project_alias_and_worktree_are_busy(self):
        worktree = self.root / 'worktree'
        self.git('worktree', 'add', '-q', '--detach', str(worktree))
        first = self.child()
        self.addCleanup(self.finish, first)
        self.assertEqual(first.stderr.readline().strip(), 'ENTERED')
        for repo in (self.repo, worktree):
            second = self.child('different-project', repo)
            out, err = second.communicate('', timeout=10)
            self.assertEqual(second.returncode, 0, err)
            self.assertEqual(json.loads(out)['status'], 'busy')
            self.assertNotIn('ENTERED', err)
        first.communicate('x', timeout=10)
        third = self.child('after-release')
        out, err = third.communicate('x', timeout=10)
        self.assertEqual(json.loads(out)['status'], 'clean')
        self.assertIn('ENTERED', err)

    def test_exception_releases_lease_and_other_repositories_can_progress(self):
        first = self.child('raise')
        self.addCleanup(self.finish, first)
        self.assertEqual(first.stderr.readline().strip(), 'ENTERED')
        other = self.root / 'independent'
        subprocess.run(['git', 'init', '-q', str(other)], check=True)
        second = self.child('independent', other)
        out, err = second.communicate('x', timeout=10)
        self.assertEqual(json.loads(out)['status'], 'clean')
        self.assertIn('ENTERED', err)
        first.communicate('x', timeout=10)
        self.assertNotEqual(first.returncode, 0)
        third = self.child('after-error')
        out, err = third.communicate('x', timeout=10)
        self.assertEqual(json.loads(out)['status'], 'clean')

    def test_busy_cli_is_distinct_from_success(self):
        sys.path.insert(0, str(SCRIPT.parent))
        self.addCleanup(sys.path.remove, str(SCRIPT.parent))
        with patch.dict(os.environ, self.env):
            spec = importlib.util.spec_from_file_location('ci_busy_fixture', SCRIPT)
            m = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(m)
        from types import SimpleNamespace
        m._do_tick_project = lambda *a, **k: {'status': 'busy'}
        m._stdout_is_the_log = lambda: False
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = m.cmd_tick(SimpleNamespace(project='fixture', dry_run=False, json=False))
        self.assertEqual(code, 3)
        self.assertIn('no build attempted', output.getvalue())
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = m.cmd_tick(SimpleNamespace(project='fixture', dry_run=False, json=True))
        self.assertEqual(code, 3)
        self.assertEqual(json.loads(output.getvalue())['status'], 'busy')

    def test_failed_git_identity_never_enters_integration_body(self):
        sys.path.insert(0, str(SCRIPT.parent))
        self.addCleanup(sys.path.remove, str(SCRIPT.parent))
        with patch.dict(os.environ, self.env):
            spec = importlib.util.spec_from_file_location('ci_lookup_fixture', SCRIPT)
            m = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(m)
        m._project_cfg = lambda *a: {'repo': str(self.repo)}
        for result in (subprocess.CompletedProcess([], 124, '', ''),
                       subprocess.CompletedProcess([], 0, '', '')):
            m._run = lambda *a, **k: result
            with patch.object(m, '_do_tick_project_locked') as body:
                self.assertEqual(m._do_tick_project('fixture')['status'], 'blocked')
                body.assert_not_called()

    def test_configuration_and_canonical_repository_are_pinned_to_lease(self):
        sys.path.insert(0, str(SCRIPT.parent))
        self.addCleanup(sys.path.remove, str(SCRIPT.parent))
        with patch.dict(os.environ, self.env):
            spec = importlib.util.spec_from_file_location('ci_retarget_fixture', SCRIPT)
            m = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(m)
        original = {'repo': str(self.repo)}
        with patch.object(m, '_project_cfg', side_effect=[original, {'repo': 'other'}]) as cfg:
            def body(project, dry=False, *, pcfg):
                original['repo'] = 'retargeted by another writer'
                self.assertEqual(pcfg['repo'], str(self.repo.resolve()))
                return {'status': 'clean'}
            m._do_tick_project_locked = body
            self.assertEqual(m._do_tick_project('fixture')['status'], 'clean')
            self.assertEqual(cfg.call_count, 1)


if __name__ == '__main__':
    unittest.main()
