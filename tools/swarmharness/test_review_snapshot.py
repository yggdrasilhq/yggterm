"""Hermetic local Git repositories and real publication/crash falsifiers."""
import fcntl
import json
import multiprocessing
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from . import review_snapshot as rs


def git(repo, *args, data=None):
    env = dict(os.environ, GIT_AUTHOR_NAME='Example', GIT_AUTHOR_EMAIL='author@example.test',
               GIT_COMMITTER_NAME='Example', GIT_COMMITTER_EMAIL='author@example.test',
               GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')
    return subprocess.check_output(['/usr/bin/git', '-C', str(repo), *args],
                                   input=data, env=env, stderr=subprocess.DEVNULL).decode().strip()


def crash_stage(root, repo, request, phase):
    store = rs.Store(root, qualified_local=True)
    def checkpoint(value):
        if value == phase:
            os._exit(71)
    store._checkpoint = checkpoint
    store.stage(repo, request)


def race_stage(root, repo, request, ready, start, out):
    store = rs.Store(root, qualified_local=True)
    ready.put(True)
    start.wait(5)
    try:
        out.put(('ok', store.stage(repo, request).artifact_id))
    except rs.Hold as e:
        out.put(('hold', str(e)))


class SnapshotTests(unittest.TestCase):
    def setUp(self):
        # Explicit physical scratch; never a RAM-backed default temporary root.
        scratch = Path.home() / '.yggterm' / 'scratchpad'
        scratch.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix='snapshot-test-', dir=scratch)
        self.base = Path(self.temp.name)
        self.repo = self.base / 'repo'
        self.repo.mkdir()
        git(self.repo, 'init', '-q')
        (self.repo / 'lib').mkdir()
        (self.repo / 'lib' / 'sample.py').write_bytes(b'print("example")\n')
        git(self.repo, 'add', '.')
        git(self.repo, 'commit', '-qm', 'Example source')
        self.commit = git(self.repo, 'rev-parse', 'HEAD')
        self.root = self.base / 'store'
        self.root.mkdir(mode=0o700)
        self.store = rs.Store(self.root, qualified_local=True)
        self.request = dict(schema=1, commit=self.commit, scope='example/source', paths=['lib/sample.py'])

    def tearDown(self):
        self.temp.cleanup()

    def stage(self, **changes):
        return self.store.stage(self.repo, dict(self.request, **changes))

    def hold(self, **changes):
        with self.assertRaises(rs.Hold):
            self.stage(**changes)

    def test_dirty_worktree_and_index_do_not_change_snapshot(self):
        original = (self.repo / 'lib/sample.py').read_bytes()
        (self.repo / 'lib/sample.py').write_bytes(b'index bytes')
        git(self.repo, 'add', '.')
        (self.repo / 'lib/sample.py').write_bytes(b'worktree bytes')
        result = self.stage()
        self.assertEqual(Path(result.files[0]).read_bytes(), original)
        self.assertEqual(result, self.stage())
        self.assertEqual(len(list(self.root.glob('provenance-*'))), 1)

    def test_same_content_different_commit_and_roots(self):
        first = self.stage()
        git(self.repo, 'commit', '--allow-empty', '-qm', 'Different provenance')
        second = self.stage(commit=git(self.repo, 'rev-parse', 'HEAD'))
        self.assertEqual(first.artifact_id, second.artifact_id)
        self.assertNotEqual(first.provenance_receipt, second.provenance_receipt)
        clone = self.base / 'other-repo'
        shutil.copytree(self.repo, clone)
        third = self.store.stage(clone, self.request)
        self.assertEqual(first.artifact_id, third.artifact_id)
        self.assertNotEqual(first.provenance_receipt, third.provenance_receipt)
        other = self.base / 'other-store'
        other.mkdir(mode=0o700)
        fourth = rs.Store(other, qualified_local=True).stage(self.repo, self.request)
        self.assertEqual(first.artifact_id, fourth.artifact_id)
        self.assertNotEqual(first.files, fourth.files)
        self.assertEqual(len(list(self.root.glob('provenance-*'))), 3)

    def test_nested_source_directories_are_private_independent_of_umask(self):
        nested = self.repo / 'tools/swarmharness/deep/example.py'
        nested.parent.mkdir(parents=True)
        nested.write_bytes(b'example source\n')
        git(self.repo, 'add', '.')
        git(self.repo, 'commit', '-qm', 'Nested source')
        previous = os.umask(0)
        try:
            result = self.stage(commit=git(self.repo, 'rev-parse', 'HEAD'),
                                paths=['tools/swarmharness/deep/example.py'])
        finally:
            os.umask(previous)
        self.assertEqual(Path(result.files[0]).read_bytes(), b'example source\n')
        source = Path(result.materialization)
        for relative in ['', 'tools', 'tools/swarmharness', 'tools/swarmharness/deep']:
            self.assertEqual((source / relative).stat().st_mode & 0o777, 0o700)

    def test_root_permission_drift_holds_before_creating_any_entry(self):
        self.root.chmod(0o777)
        try:
            with self.assertRaisesRegex(rs.Hold, 'unsafe_storage'):
                self.stage()
            self.assertEqual(list(self.root.iterdir()), [])
        finally:
            self.root.chmod(0o700)

    def test_existing_snapshot_retry_refuses_root_permission_drift(self):
        self.stage()
        before = {str(p.relative_to(self.root)): p.read_bytes()
                  for p in self.root.rglob('*') if p.is_file()}
        self.root.chmod(0o777)
        try:
            with self.assertRaisesRegex(rs.Hold, 'unsafe_storage'):
                self.stage()
            self.assertEqual(before, {str(p.relative_to(self.root)): p.read_bytes()
                                      for p in self.root.rglob('*') if p.is_file()})
        finally:
            self.root.chmod(0o700)

    def test_root_replacement_and_symlink_hold_without_redirected_writes(self):
        original = self.base / 'enrolled-root'
        self.root.rename(original)
        self.root.mkdir(mode=0o700)
        with self.assertRaisesRegex(rs.Hold, 'root_identity_changed'):
            self.stage()
        self.assertEqual(list(self.root.iterdir()), [])
        self.assertEqual(list(original.iterdir()), [])
        replacement = self.base / 'replacement-root'
        self.root.rename(replacement)
        self.root.symlink_to(replacement, target_is_directory=True)
        with self.assertRaises(rs.Hold):
            self.stage()
        self.assertEqual(list(replacement.iterdir()), [])
        self.assertEqual(list(original.iterdir()), [])

    def test_sha256_git_repository_content_identity(self):
        first = self.stage()
        repo = self.base / 'sha256-repo'
        repo.mkdir()
        git(repo, 'init', '-q', '--object-format=sha256')
        (repo / 'lib').mkdir()
        (repo / 'lib/sample.py').write_bytes(b'print("example")\n')
        git(repo, 'add', '.')
        git(repo, 'commit', '-qm', 'Example source')
        commit = git(repo, 'rev-parse', 'HEAD')
        self.assertEqual(len(commit), 64)
        second = self.store.stage(repo, dict(self.request, commit=commit))
        self.assertEqual(first.artifact_id, second.artifact_id)
        self.assertNotEqual(first.provenance_receipt, second.provenance_receipt)

    def test_bytes_modes_names_scope_change_identity(self):
        first = self.stage()
        (self.repo / 'lib/sample.py').write_bytes(b'changed')
        git(self.repo, 'add', '.')
        git(self.repo, 'commit', '-qm', 'Change bytes')
        second = self.stage(commit=git(self.repo, 'rev-parse', 'HEAD'))
        git(self.repo, 'update-index', '--chmod=+x', 'lib/sample.py')
        git(self.repo, 'commit', '-qm', 'Change mode')
        third = self.stage(commit=git(self.repo, 'rev-parse', 'HEAD'))
        self.assertEqual(os.stat(third.files[0]).st_mode & 0o777, 0o500)
        self.assertEqual(len({first.artifact_id, second.artifact_id, third.artifact_id}), 3)
        self.assertNotEqual(first.artifact_id, self.stage(scope='other').artifact_id)
        (self.repo / 'other.py').write_bytes(b'print("example")\n')
        git(self.repo, 'add', '.')
        git(self.repo, 'commit', '-qm', 'Different name')
        self.assertNotEqual(first.artifact_id, self.stage(commit=git(self.repo, 'rev-parse', 'HEAD'), paths=['other.py']).artifact_id)

    def test_closed_request_schema_and_unsafe_paths(self):
        for changes in [dict(extra=1), dict(schema=2), dict(schema=True), dict(commit='HEAD'),
                        dict(commit=self.commit+'^'), dict(paths=[]), dict(paths=['x', 'x']),
                        dict(paths=['a', 'a/b']), dict(paths=['/escape']), dict(paths=['../escape']),
                        dict(paths=['a//b']), dict(paths=['a/./b']), dict(paths=['a\\b']),
                        dict(paths=['.git/config']), dict(paths=['x\n']), dict(scope='')]:
            with self.subTest(changes=changes):
                self.hold(**changes)

    def test_symlink_directory_submodule_missing_and_ancestor(self):
        (self.repo / 'link').symlink_to('/outside')
        git(self.repo, 'add', '.')
        git(self.repo, 'update-index', '--add', '--cacheinfo', '160000,'+self.commit+',module')
        git(self.repo, 'commit', '-qm', 'Unsupported entries')
        commit = git(self.repo, 'rev-parse', 'HEAD')
        for path in ['link', 'link/secret', 'module', 'module/secret', 'lib', 'absent']:
            self.hold(commit=commit, paths=[path])

    def test_unknown_executable_mode_and_missing_tree_hold(self):
        blob = git(self.repo, 'rev-parse', self.commit+':lib/sample.py')
        raw = b'100664 odd.py\0' + bytes.fromhex(blob)
        tree = git(self.repo, 'hash-object', '--literally', '-t', 'tree', '-w', '--stdin', data=raw)
        commit = git(self.repo, 'commit-tree', tree, data=b'Unknown mode\n')
        self.hold(commit=commit, paths=['odd.py'])
        tree_path = self.repo / '.git/objects' / tree[:2] / tree[2:]
        tree_path.unlink()
        self.hold(commit=commit, paths=['odd.py'])

    def test_partial_and_extra_artifact_entries_hold(self):
        first = self.stage()
        extra = Path(first.materialization).parent / 'extra'
        extra.write_bytes(b'preserved')
        self.hold()
        self.assertEqual(extra.read_bytes(), b'preserved')
        extra.unlink()
        Path(first.files[0]).unlink()
        self.hold()
        self.assertFalse(Path(first.files[0]).exists())

    def test_special_file_and_manifest_symlink_hold(self):
        first = self.stage()
        path = Path(first.files[0])
        path.unlink()
        os.mkfifo(path, 0o600)
        self.hold()
        self.assertTrue(path.exists())
        path.unlink()
        manifest = Path(first.materialization).parent / 'manifest.json'
        target = self.base / 'manifest-target'
        target.write_bytes(manifest.read_bytes())
        manifest.unlink()
        manifest.symlink_to(target)
        self.hold()
        self.assertTrue(manifest.is_symlink())

    def test_missing_and_corrupt_blob(self):
        blob = git(self.repo, 'rev-parse', self.commit+':lib/sample.py')
        location = self.repo / '.git/objects' / blob[:2] / blob[2:]
        saved = location.read_bytes()
        location.unlink()
        self.hold()
        location.write_bytes(saved[:-2] + b'xx')
        self.hold()

    def test_caps(self):
        for target, cap in [('MAX_FILES', 0), ('MAX_BLOB', 1), ('MAX_TOTAL', 1),
                            ('MAX_OBJECT', 1), ('MAX_MANIFEST', 1), ('TIMEOUT', 0)]:
            with self.subTest(target=target), patch.object(rs, target, cap):
                self.hold()

    def test_tampering_retains_exact_bytes(self):
        first = self.stage()
        path = Path(first.files[0])
        path.chmod(0o600)
        path.write_bytes(b'tampered')
        self.hold()
        self.assertEqual(path.read_bytes(), b'tampered')

    def test_unknown_manifest_and_layout_retained(self):
        first = self.stage()
        manifest = Path(first.materialization).parent / 'manifest.json'
        manifest.chmod(0o600)
        raw = json.loads(manifest.read_bytes())
        raw['schema'] = 99
        data = rs._canonical(raw)
        manifest.write_bytes(data)
        manifest.chmod(0o400)
        self.hold()
        self.assertEqual(manifest.read_bytes(), data)

    def test_unknown_empty_artifact_never_replaced(self):
        manifest, _, _, _ = rs._sources(self.repo, self.request, self.request['paths'])
        dest = self.root / rs._sha(rs._canonical(manifest))
        dest.mkdir(mode=0o700)
        self.hold()
        self.assertEqual(list(dest.iterdir()), [])

    def test_nofollow_leaf_paths_and_private_root(self):
        target = self.base / 'target'
        target.write_bytes(b'untouched')
        lock = self.root / '.lock'
        lock.symlink_to(target)
        self.hold()
        self.assertEqual(target.read_bytes(), b'untouched')
        lock.unlink()
        first = self.stage()
        path = Path(first.files[0])
        path.unlink()
        path.symlink_to(target)
        self.hold()
        self.assertEqual(target.read_bytes(), b'untouched')
        self.root.chmod(0o755)
        with self.assertRaises(rs.Hold):
            rs.Store(self.root, qualified_local=True)

    def test_root_alias_resolves_once_and_requires_qualification(self):
        alias = self.base / 'alias'
        alias.symlink_to(self.root, target_is_directory=True)
        store = rs.Store(alias, qualified_local=True)
        alias.unlink()
        alias.symlink_to(self.repo, target_is_directory=True)
        self.assertTrue(store.stage(self.repo, self.request).materialization.startswith(str(self.root)))
        with self.assertRaises(rs.Hold):
            rs.Store(self.root)

    def test_lock_contention(self):
        with open(self.root / '.lock', 'w') as f:
            os.chmod(f.name, 0o600)
            fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaisesRegex(rs.Hold, '^lock_busy$'):
                self.stage()
        self.stage()

    def test_concurrent_publish_and_retry(self):
        ctx = multiprocessing.get_context('fork')
        ready, out, start = ctx.Queue(), ctx.Queue(), ctx.Event()
        children = [ctx.Process(target=race_stage, args=(self.root, self.repo, self.request, ready, start, out)) for _ in range(2)]
        for p in children:
            p.start()
        for _ in children:
            ready.get(timeout=5)
        start.set()
        results = [out.get(timeout=15) for _ in children]
        for p in children:
            p.join(5)
            self.assertEqual(p.exitcode, 0)
        self.assertTrue(any(r[0] == 'ok' for r in results))
        self.assertTrue(all(r[0] == 'ok' or r == ('hold', 'lock_busy') for r in results))
        result = self.stage()
        self.assertEqual(len([p for p in self.root.iterdir() if len(p.name) == 64]), 1)
        self.assertEqual(result, self.stage())

    def test_real_process_crash_before_and_after_visibility(self):
        for phase in ('before_publish', 'after_publish', 'durable'):
            with self.subTest(phase=phase):
                root = self.base / phase
                root.mkdir(mode=0o700)
                p = multiprocessing.get_context('fork').Process(target=crash_stage, args=(root, self.repo, self.request, phase))
                p.start()
                p.join(15)
                self.assertEqual(p.exitcode, 71)
                artifacts = [p for p in root.iterdir() if len(p.name) == 64]
                self.assertEqual(bool(artifacts), phase != 'before_publish')
                if phase == 'before_publish':
                    self.assertEqual(len(list(root.glob('.stage-*'))), 1)
                store = rs.Store(root, qualified_local=True)
                result = store.stage(self.repo, self.request)
                self.assertEqual(result, store.stage(self.repo, self.request))

    def test_post_visibility_io_failure_reports_uncertainty(self):
        def fail(phase):
            if phase == 'after_publish':
                raise OSError('injected')
        with patch.object(self.store, '_checkpoint', fail):
            with self.assertRaisesRegex(rs.Hold, '^durability_uncertain$'):
                self.stage()
        self.assertEqual(self.stage(), self.stage())

    def test_git_environment_does_not_inherit_replacements_or_commands(self):
        marker = self.base / 'hook-ran'
        hook = self.repo / '.git/hooks/post-checkout'
        hook.write_text('#!/bin/sh\ntouch '+str(marker)+'\n')
        hook.chmod(0o755)
        with patch.dict(os.environ, {'GIT_DIR': '/nonexistent', 'GIT_WORK_TREE': '/nonexistent',
                                     'GIT_EXTERNAL_DIFF': '/nonexistent', 'GIT_CONFIG_COUNT': '999'}):
            self.stage()
        self.assertFalse(marker.exists())

    def test_provenance_corruption_is_retained(self):
        first = self.stage()
        path = Path(first.provenance_receipt)
        path.chmod(0o600)
        path.write_bytes(b'{"schema":99}')
        path.chmod(0o400)
        self.hold()
        self.assertEqual(path.read_bytes(), b'{"schema":99}')


if __name__ == '__main__':
    unittest.main()
