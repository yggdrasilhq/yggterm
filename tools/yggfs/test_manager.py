import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import sqlite3
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
spec = importlib.util.spec_from_file_location("ygg_fs_under_test",
    Path(__file__).resolve().parents[2]/".agents/skills/yggterm-agent-fleet/ygg-fs.py")
fs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fs)

class Roots(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.home = Path(self.tmp.name)

    def resolve(self, **kw):
        return fs.resolve(home=self.home, env={}, **kw)

    def init(self, **kw):
        return fs.initialize(home=self.home, env={}, **kw)

    def test_discovery_never_creates_fresh_state(self):
        d = self.resolve()
        self.assertFalse(d['initialized'])
        self.assertEqual(d['selection'], 'fresh-uninitialized')
        self.assertFalse((self.home/'.ygg').exists())
        self.assertFalse((self.home/'.config').exists())

    def test_fresh_init_uses_new_root_and_preserves_identity(self):
        first = self.init()
        second = self.init()
        self.assertEqual(first['root_id'], second['root_id'])
        self.assertTrue((self.home/'.ygg/root.json').is_file())
        self.assertTrue((self.home/'.yggterm').is_symlink())
        self.assertTrue((self.home/'.ygg').samefile(self.home/'.yggterm'))

    def test_legacy_alias_preserves_live_lock_inode_and_sqlite_wal(self):
        import fcntl
        old = self.home/'.yggterm'
        old.mkdir()
        with (old/'live.lock').open('a+') as held:
            fcntl.flock(held, fcntl.LOCK_EX|fcntl.LOCK_NB)
            before = os.fstat(held.fileno())
            db = sqlite3.connect(old/'state.sqlite')
            self.addCleanup(db.close)
            db.execute('pragma journal_mode=wal')
            db.execute('create table facts(value text)')
            db.execute("insert into facts values('before')")
            db.commit()
            cfg = old/'swarm-config.yaml'
            cfg.write_bytes(b'preserve-policy-bytes\n')
            result = self.init()
            new = self.home/'.ygg'
            self.assertTrue(new.is_symlink())
            self.assertTrue(new.samefile(old))
            self.assertEqual((new/'live.lock').stat().st_ino, before.st_ino)
            with (new/'live.lock').open('a+') as competitor:
                with self.assertRaises(BlockingIOError):
                    fcntl.flock(competitor, fcntl.LOCK_EX|fcntl.LOCK_NB)
            db.execute("insert into facts values('after')")
            db.commit()
            other = sqlite3.connect(new/'state.sqlite')
            self.addCleanup(other.close)
            self.assertEqual(other.execute('select value from facts').fetchall(), [('before',),('after',)])
            self.assertEqual(cfg.read_bytes(), b'preserve-policy-bytes\n')
            self.assertFalse(result['physical_relocation_complete'])
            self.assertFalse(result['subtree_conversion_complete'])

    def test_two_real_roots_hold_and_never_merge(self):
        for n in ['.ygg','.yggterm']:
            (self.home/n).mkdir()
            (self.home/n/'custom').write_text(n)
        with self.assertRaisesRegex(fs.Held, 'two independent'):
            self.init()
        self.assertEqual((self.home/'.yggterm/custom').read_text(), '.yggterm')
        self.assertFalse((self.home/'.ygg/root.json').exists())

    def test_broken_cyclic_and_escaping_aliases_hold(self):
        new = self.home/'.ygg'
        for target in ['absent','.ygg']:
            new.symlink_to(target)
            with self.assertRaises(fs.Held): self.resolve()
            new.unlink()
        with tempfile.TemporaryDirectory() as outside:
            new.symlink_to(outside)
            with self.assertRaisesRegex(fs.Held, 'escapes'):
                self.resolve()

    def test_conflicting_overrides_hold_same_identity_is_accepted(self):
        (self.home/'.yggterm').mkdir()
        self.init()
        ok = fs.resolve(home=self.home, env={'YGG_ROOT':str(self.home/'.ygg'),
                                            'YGGTERM_HOME':str(self.home/'.yggterm')})
        self.assertTrue(ok['same_legacy_store'])
        with self.assertRaisesRegex(fs.Held, 'conflicting'):
            fs.resolve(home=self.home, env={'YGG_ROOT':str(self.home/'.ygg'),
                                           'YGGTERM_HOME':str(self.home/'other')})
        with self.assertRaisesRegex(fs.Held, 'empty'):
            fs.resolve(home=self.home, env={'YGG_ROOT':''})

    def test_unreadable_or_foreign_root_marker_never_rewritten(self):
        root = self.home/'.yggterm'
        root.mkdir()
        (root/'root.json').write_text('{broken')
        with self.assertRaises(fs.Held): self.init()
        self.assertEqual((root/'root.json').read_text(), '{broken')

    def test_locator_drift_and_missing_root_hold_without_genesis(self):
        d = self.init()
        locator = self.home/'.config/ygg/root-location.json'
        content = json.loads(locator.read_text())
        content['root_id'] = '00000000-0000-0000-0000-000000000000'
        locator.write_text(json.dumps(content))
        with self.assertRaisesRegex(fs.Held, 'disagree'):
            self.resolve()

    def test_failed_locator_write_recovers_same_initialized_store(self):
        (self.home/'.yggterm').mkdir()
        original = fs._atomic_json
        def fail_locator(path, doc, **kwargs):
            if path.name == 'root-location.json': raise OSError('simulated full disk')
            return original(path,doc,**kwargs)
        with patch.object(fs,'_atomic_json',side_effect=fail_locator):
            with self.assertRaises(OSError): self.init()
        first = self.resolve()['root_id']
        self.assertEqual(self.init()['root_id'],first)
        self.assertTrue((self.home/'.ygg').samefile(self.home/'.yggterm'))

    def test_isolated_explicit_root_does_not_change_default_locator(self):
        custom = self.home/'test-store'
        self.init(explicit=str(custom))
        self.assertFalse((self.home/'.config/ygg/root-location.json').exists())
        self.assertEqual(self.resolve()['selection'],'fresh-uninitialized')

    def test_interrupted_fresh_alias_is_repaired_on_every_replay(self):
        original = fs.os.symlink
        def fail_legacy(target, name, **kwargs):
            if str(name) == '.yggterm': raise OSError('interrupted alias publication')
            return original(target, name, **kwargs)
        with patch.object(fs.os, 'symlink', side_effect=fail_legacy):
            with self.assertRaises(OSError): self.init()
        before = (self.home/'.ygg/root.json').read_bytes()
        self.assertFalse((self.home/'.yggterm').exists())
        self.init()
        self.assertTrue((self.home/'.yggterm').samefile(self.home/'.ygg'))
        self.assertEqual(before, (self.home/'.ygg/root.json').read_bytes())
        (self.home/'.yggterm').unlink()
        self.init()
        self.assertTrue((self.home/'.yggterm').samefile(self.home/'.ygg'))

    def test_root_identity_create_once_cannot_overwrite_raced_winner(self):
        root=self.home/'explicit'
        root.mkdir()
        path=root/'root.json'
        winner={'identity':'first'}
        self.assertTrue(fs._atomic_json(path, winner, create_once=True))
        before=path.read_bytes()
        self.assertFalse(fs._atomic_json(path, {'identity':'second'}, create_once=True))
        self.assertEqual(before, path.read_bytes())

    def test_legacy_locator_missing_canonical_alias_recovers_only_matching_store(self):
        (self.home/'.yggterm').mkdir()
        self.init()
        before=(self.home/'.yggterm/root.json').read_bytes()
        (self.home/'.ygg').unlink()
        with self.assertRaisesRegex(fs.Held,'unavailable'): self.resolve()
        self.init()
        self.assertTrue((self.home/'.ygg').samefile(self.home/'.yggterm'))
        self.assertEqual(before,(self.home/'.ygg/root.json').read_bytes())
        (self.home/'.ygg').unlink()
        locator=self.home/'.config/ygg/root-location.json'
        doc=json.loads(locator.read_text())
        doc['root_id']='00000000-0000-0000-0000-000000000000'
        locator.write_text(json.dumps(doc))
        with self.assertRaisesRegex(fs.Held,'disagrees'): self.init()
        self.assertFalse((self.home/'.ygg').exists())
        self.assertEqual(before,(self.home/'.yggterm/root.json').read_bytes())

    def test_interrupted_alias_parent_fsync_is_retried_before_locator(self):
        original_sync=fs.os.fsync
        original_link=fs.os.symlink
        armed=[False]
        home_inode=self.home.stat().st_ino
        def link(*args,**kwargs):
            result=original_link(*args,**kwargs)
            armed[0]=True
            return result
        def fail_once(fd):
            if armed[0] and os.fstat(fd).st_ino==home_inode:
                armed[0]=False
                raise OSError('interrupted alias directory sync')
            return original_sync(fd)
        with patch.object(fs.os,'symlink',side_effect=link),patch.object(fs.os,'fsync',side_effect=fail_once):
            with self.assertRaises(OSError): self.init()
        self.assertTrue((self.home/'.yggterm').is_symlink())
        self.assertFalse((self.home/'.config/ygg/root-location.json').exists())
        synced=[]
        def record_sync(fd):
            if os.fstat(fd).st_ino==home_inode: synced.append(True)
            return original_sync(fd)
        with patch.object(fs.os,'fsync',side_effect=record_sync): self.init()
        self.assertTrue(synced)

    def test_different_xdg_profiles_share_physical_root_lock_and_preserve_winner(self):
        root=self.home/'explicit'
        script=Path(fs.__file__)
        code=("import importlib.util,json,sys;from pathlib import Path;"
              f"s=importlib.util.spec_from_file_location('f',{str(script)!r});"
              "f=importlib.util.module_from_spec(s);s.loader.exec_module(f);"
              f"home=Path({str(self.home)!r});env={{'XDG_CONFIG_HOME':sys.argv[1]}};"
              f"root={str(root)!r}\n"
              "try: print(json.dumps(f.initialize(home=home,env=env,explicit=root)))\n"
              "except f.Held: sys.exit(75)\n")
        configs=[str(self.home/f'profile-{i}') for i in range(2)]
        workers=[subprocess.Popen([sys.executable,'-c',code,c],stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE,text=True) for c in configs]
        rows=[]
        for w in workers:
            out,err=w.communicate(timeout=10)
            self.assertIn(w.returncode,[0,75],err)
            rows.append((w.returncode,out))
        self.assertEqual(sorted(row[0] for row in rows),[0,75])
        winner=next(i for i,row in enumerate(rows) if row[0]==0)
        record=json.loads(rows[winner][1])
        self.assertEqual(fs.resolve(home=self.home,env={'XDG_CONFIG_HOME':configs[winner]},
                                    explicit=str(root))['root_id'],record['root_id'])

    def test_gc_output_rejects_parent_symlink_without_external_writes(self):
        self.init()
        with tempfile.TemporaryDirectory() as external:
            (self.home/'.ygg/gc').symlink_to(external)
            with self.assertRaises((fs.Held,OSError)):
                fs.gc_plan(home=self.home,env={})
            self.assertEqual(list(Path(external).iterdir()),[])

    def test_gc_output_rejects_existing_leaf_symlink(self):
        self.init()
        plan=fs.gc_plan(home=self.home,env={})
        # A stable second plan permits targeting its exact output leaf.
        plan=fs.gc_plan(home=self.home,env={})
        output=self.home/'.ygg/gc/plans'/(plan['plan_id']+'.json')
        outside=self.home/'outside'
        outside.write_text('retained')
        output.unlink()
        output.symlink_to(outside)
        with self.assertRaises(fs.Held): fs.gc_plan(home=self.home,env={})
        self.assertEqual(outside.read_text(),'retained')

    def test_bootstrap_syncs_directory_entries_and_alias_before_locator(self):
        events=[]
        original_sync=fs.os.fsync
        original_link=fs.os.symlink
        original_atomic=fs._atomic_json
        def sync(fd):
            events.append('directory-sync' if stat.S_ISDIR(os.fstat(fd).st_mode) else 'file-sync')
            return original_sync(fd)
        def link(*args,**kwargs):
            events.append('alias')
            return original_link(*args,**kwargs)
        def atomic(path,doc,**kwargs):
            events.append(path.name)
            return original_atomic(path,doc,**kwargs)
        with patch.object(fs.os,'fsync',side_effect=sync),patch.object(fs.os,'symlink',side_effect=link),patch.object(fs,'_atomic_json',side_effect=atomic):
            self.init()
        self.assertGreater(events.count('directory-sync'),3)
        self.assertLess(events.index('host.json'),events.index('root.json'))
        self.assertLess(events.index('root.json'),events.index('alias'))
        self.assertIn('directory-sync',events[events.index('alias')+1:events.index('root-location.json')])

    def test_locations_are_ids_not_subtree_migrations(self):
        self.init()
        known = fs.location('kernel.swarm',home=self.home,env={})
        self.assertTrue(known['location_path'].endswith('/swarm'))
        self.assertTrue(known['target_path'].endswith('/kernel/swarm'))
        self.assertEqual(known['conversion'],'owner-migration-required')
        app = fs.location('app.example-ui',home=self.home,env={})
        self.assertTrue(app['location_path'].endswith('/libyggterm/example-ui'))
        with self.assertRaises(fs.Held): fs.location('app.../escape',home=self.home,env={})

    def test_imported_root_identity_is_not_a_local_enrollment(self):
        self.init()
        other = self.home/'foreign-home'
        other.mkdir()
        (other/'.ygg').mkdir()
        (other/'.ygg/root.json').write_bytes((self.home/'.ygg/root.json').read_bytes())
        with self.assertRaisesRegex(fs.Held, 'different or uninitialized host'):
            fs.resolve(home=other,env={})

    def test_xdg_bootstrap_location_is_honored(self):
        config=self.home/'settings'
        env={'XDG_CONFIG_HOME':str(config)}
        fs.initialize(home=self.home,env=env)
        self.assertTrue((config/'ygg/root-location.json').is_file())
        self.assertTrue(fs.resolve(home=self.home,env=env)['initialized'])
        self.assertFalse((self.home/'.config/ygg').exists())

    def test_concurrent_initializers_share_one_identity(self):
        (self.home/'.yggterm').mkdir()
        script = Path(fs.__file__)
        code = ("import importlib.util,json;from pathlib import Path;"
                f"s=importlib.util.spec_from_file_location('f',{str(script)!r});"
                "f=importlib.util.module_from_spec(s);s.loader.exec_module(f);"
                f"print(json.dumps(f.initialize(home=Path({str(self.home)!r}),env={{}})))")
        workers = [subprocess.Popen([sys.executable,'-c',code],stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE,text=True) for _ in range(4)]
        rows=[]
        for worker in workers:
            out,err=worker.communicate(timeout=10)
            self.assertEqual(worker.returncode,0,err)
            rows.append(json.loads(out))
        self.assertEqual(len({r['root_id'] for r in rows}),1)
        self.assertTrue((self.home/'.ygg').samefile(self.home/'.yggterm'))

    def test_metadata_links_and_nonregular_files_hold(self):
        root=self.home/'.yggterm'
        root.mkdir()
        outside=self.home/'foreign-metadata'
        outside.write_text('{}')
        (root/'root.json').symlink_to(outside)
        with self.assertRaisesRegex(fs.Held,'not a regular'):
            self.init()
        self.assertEqual(outside.read_text(),'{}')
        (root/'root.json').unlink()
        os.mkfifo(root/'root.json')
        with self.assertRaisesRegex(fs.Held,'not a regular'):
            self.resolve()

    def test_gc_plan_only_retains_all_objects_and_never_walks_links(self):
        self.init()
        root=self.home/'.ygg'
        (root/'custom').write_text('keep')
        (root/'auth').mkdir()
        (root/'auth/private').write_text('fixture-secret-never-read')
        (root/'external').symlink_to(self.home.parent)
        before=(root/'custom').read_bytes()
        plan=fs.gc_plan(home=self.home,env={})
        self.assertEqual(plan['eligible_objects'],0)
        self.assertEqual(plan['reclaimed_bytes'],0)
        self.assertTrue(all(r['action']=='retain' for r in plan['objects']))
        self.assertEqual((root/'custom').read_bytes(),before)
        self.assertNotIn('private',json.dumps(plan))
        self.assertTrue((root/'gc/plans'/(plan['plan_id']+'.json')).is_file())

    def test_gc_apply_is_a_typed_nonmutation(self):
        out=io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(fs.cli(['gc','apply','--json']),75)
        self.assertEqual(json.loads(out.getvalue())['status'],'HOLD')

if __name__=='__main__': unittest.main()
