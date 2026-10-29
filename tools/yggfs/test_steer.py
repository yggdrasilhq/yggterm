import copy
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
import uuid
from unittest.mock import patch

spec=importlib.util.spec_from_file_location('steer_under_test',
    Path(__file__).resolve().parents[2]/'.agents/skills/yggterm-agent-fleet/ygg-steer.py')
steer=importlib.util.module_from_spec(spec)
spec.loader.exec_module(steer)


class Steering(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.home=Path(self.tmp.name)
        self.kw={'home':self.home,'env':{}}
        steer.fs.initialize(**self.kw)
        steer.initialize(**self.kw)

    def test_fresh_seed_is_self_contained_and_no_target_is_stamped_by_init(self):
        result=steer.initialize(**self.kw)
        self.assertFalse(result['fleet_stamped'])
        self.assertFalse(result['legacy_repository_retired'])
        self.assertFalse((self.home/'.codex/AGENTS.md').exists())
        p=steer.plan('codex',**self.kw)
        self.assertIn('ygg memory sync',p['output'])
        self.assertIn('independent acceptance',p['output'])

    def test_stamped_target_matches_and_plan_replay_does_not_rewrite(self):
        p=steer.plan('codex',**self.kw)
        r=steer.stamp(p,**self.kw)
        self.assertEqual(r['status'],'stamped')
        target=Path(p['target']);before=target.stat()
        again=steer.plan('codex',**self.kw)
        self.assertFalse(again['changed'])
        self.assertEqual(steer.stamp(again,**self.kw)['status'],'unchanged')
        self.assertEqual(before.st_ino,target.stat().st_ino)

    def test_private_overlay_survives_repeated_initialization(self):
        policy=self.home/'.ygg/steer/overlays/policy.json'
        doc=json.loads(policy.read_text());doc['suffix']='Operator-local fixture policy.'
        policy.write_text(json.dumps(doc))
        before=policy.read_bytes()
        steer.initialize(**self.kw)
        self.assertEqual(policy.read_bytes(),before)
        self.assertIn('Operator-local fixture policy.',steer.plan('codex',**self.kw)['output'])

    def test_foreign_memory_guard_and_user_bytes_survive_exactly(self):
        target=self.home/'.codex/AGENTS.md';target.parent.mkdir()
        original=b'User text\r\n<!-- BEGIN yggterm-memory -->\r\nowned by memory\r\n<!-- END yggterm-memory -->\r\n'
        target.write_bytes(original)
        p=steer.plan('codex',**self.kw)
        steer.stamp(p,**self.kw)
        self.assertTrue(target.read_bytes().startswith(original))
        self.assertEqual(target.read_bytes().count(b'<!-- BEGIN yggterm-memory -->'),1)

    def test_unknown_harness_and_collision_alias_refused(self):
        for harness in ['unknown','pi','../escape']:
            with self.assertRaises(steer.fs.Held):steer.plan(harness,**self.kw)

    def test_unqualified_platform_is_a_typed_hold(self):
        with patch.object(steer.os,'name','nt'):
            with self.assertRaisesRegex(steer.fs.Held,'only on POSIX'):steer.initialize(**self.kw)
            with self.assertRaisesRegex(steer.fs.Held,'only on POSIX'):steer.plan('codex',**self.kw)

    def test_malformed_guards_and_nested_foreign_region_hold(self):
        fixtures=[steer.BEGIN,steer.END,steer.BEGIN+steer.END+steer.BEGIN+steer.END,
                  steer.END+steer.BEGIN,
                  '<!-- BEGIN yggterm-memory -->unfinished',
                  '<!-- END yggterm-memory -->reversed',
                  '<!-- BEGIN yggterm-memory',
                  '<!-- BEGIN ygg-steer:root',
                  '<!-- BEGIN foreign.name -->unclosed',
                  '<!-- BEGIN foreign/name -->unclosed',
                  steer.BEGIN+'<!-- BEGIN yggterm-memory -->x<!-- END yggterm-memory -->'+steer.END]
        for text in fixtures:
            with self.subTest(text=text),self.assertRaises(steer.fs.Held):steer.render(text.encode(),'replacement')

    def test_private_suffix_cannot_insert_another_managed_block(self):
        policy=self.home/'.ygg/steer/overlays/policy.json'
        policy.write_text(json.dumps({'schema':1,'suffix':steer.BEGIN}))
        with self.assertRaises(steer.fs.Held):steer.plan('codex',**self.kw)

    def test_stale_plan_refuses_target_or_policy_drift_without_overwrite(self):
        p=steer.plan('codex',**self.kw)
        target=Path(p['target']);target.parent.mkdir();target.write_text('new user instruction')
        with self.assertRaises(steer.fs.Held):steer.stamp(p,**self.kw)
        self.assertEqual(target.read_text(),'new user instruction')
        p=steer.plan('codex',**self.kw)
        policy=self.home/'.ygg/steer/overlays/policy.json'
        policy.write_text(json.dumps({'schema':1,'suffix':'changed local fixture'}))
        with self.assertRaises(steer.fs.Held):steer.stamp(p,**self.kw)
        self.assertEqual(target.read_text(),'new user instruction')

    def test_plan_payload_tampering_refused(self):
        p=steer.plan('codex',**self.kw);p['output']='invented acceptance'
        with self.assertRaises(steer.fs.Held):steer.stamp(p,**self.kw)
        self.assertFalse(Path(p['target']).exists())

    def test_normalized_target_collisions_and_unsafe_unused_mappings_hold(self):
        for mapping in ['./.codex/AGENTS.md','.codex//AGENTS.md','.codex/AGENTS.md','../escape']:
            active=self.home/'.ygg/steer/active.json'
            old=json.loads(active.read_text())
            seed=json.loads((self.home/'.ygg/steer/seeds'/(old['seed']+'.json')).read_text())
            seed['targets']['claude']=mapping
            key=steer.digest(steer._encoded(seed))
            (self.home/'.ygg/steer/seeds'/(key+'.json')).write_text(json.dumps(seed))
            active.write_text(json.dumps({'schema':1,'seed':key}))
            with self.subTest(mapping=mapping),self.assertRaises(steer.fs.Held):steer.plan('codex',**self.kw)

    def test_custom_memory_owner_configuration_holds_before_publication(self):
        p=steer.plan('codex',**self.kw)
        with self.assertRaisesRegex(steer.fs.Held,'native-writer fence'):
            steer.stamp(p,home=self.home,env={'YGGTERM_MEMORY_ROOT':str(self.home/'other-hub')})
        self.assertFalse(Path(p['target']).exists())

    def test_existing_hardlinked_targets_are_one_identity_and_hold(self):
        first=self.home/'.codex/AGENTS.md';first.parent.mkdir();first.write_text('retained')
        second=self.home/'.claude/CLAUDE.md';second.parent.mkdir();os.link(first,second)
        with self.assertRaisesRegex(steer.fs.Held,'existing file identity'):steer.plan('codex',**self.kw)
        self.assertEqual(first.read_text(),'retained')

    def test_custom_steering_store_keeps_native_home_memory_fence(self):
        custom=self.home/'steering-sandbox'
        steer.fs.initialize(home=self.home,env={},explicit=str(custom))
        steer.initialize(home=self.home,env={},explicit=str(custom))
        p=steer.plan('codex',home=self.home,env={},explicit=str(custom))
        original=steer.fs._file_lock
        locks=[]
        def lock(directory,name):
            locks.append((Path(directory),name))
            return original(directory,name)
        with patch.object(steer.fs,'_file_lock',side_effect=lock):
            steer.stamp(p,home=self.home,env={},explicit=str(custom))
        self.assertIn(((self.home/'.ygg/memory').resolve(),'.ygg-memory.lock'),locks)
        self.assertNotIn((custom/'memory','.ygg-memory.lock'),locks)

    def test_custom_steering_store_cannot_enroll_an_untrusted_native_hub(self):
        for case in ['uninitialized','foreign','escaping']:
            home=self.home/case;home.mkdir()
            custom=home/'private-steer'
            steer.fs.initialize(home=home,env={},explicit=str(custom))
            steer.initialize(home=home,env={},explicit=str(custom))
            native=home/'.yggterm'
            if case=='escaping':
                external=self.home/'external-profile';external.mkdir()
                native.symlink_to(external)
                protected=external
            else:
                native.mkdir()
                protected=native
                if case=='foreign':
                    marker=json.loads((custom/'root.json').read_text())
                    marker['host_id']=str(uuid.uuid4())
                    (native/'root.json').write_text(json.dumps(marker))
            p=steer.plan('codex',home=home,env={},explicit=str(custom))
            with self.subTest(case=case),self.assertRaises(steer.fs.Held):
                steer.stamp(p,home=home,env={},explicit=str(custom))
            self.assertFalse(Path(p['target']).exists())
            self.assertFalse((protected/'memory').exists())

    def test_target_and_parent_symlinks_hold_without_external_writes(self):
        outside=self.home/'outside';outside.write_text('retained')
        target=self.home/'.codex/AGENTS.md';target.parent.mkdir();target.symlink_to(outside)
        with self.assertRaises((steer.fs.Held,OSError)):steer.plan('codex',**self.kw)
        target.unlink();target.parent.rmdir();target.parent.symlink_to(self.home)
        p=steer.plan('codex',**self.kw)
        with self.assertRaises((steer.fs.Held,OSError)):steer.stamp(p,**self.kw)
        self.assertEqual(outside.read_text(),'retained')

    def test_publication_failure_before_replace_keeps_old_target(self):
        target=self.home/'.codex/AGENTS.md';target.parent.mkdir();target.write_text('retained')
        p=steer.plan('codex',**self.kw)
        with patch.object(steer.os,'replace',side_effect=OSError('simulated publication failure')):
            with self.assertRaises(OSError):steer.stamp(p,**self.kw)
        self.assertEqual(target.read_text(),'retained')

    def test_immutable_seed_mismatch_is_held(self):
        active=json.loads((self.home/'.ygg/steer/active.json').read_text())
        path=self.home/'.ygg/steer/seeds'/(active['seed']+'.json')
        doc=json.loads(path.read_text());doc['root_markdown']='unexpected generation bytes';path.write_text(json.dumps(doc))
        with self.assertRaises(steer.fs.Held):steer.plan('codex',**self.kw)


if __name__=='__main__':unittest.main()
