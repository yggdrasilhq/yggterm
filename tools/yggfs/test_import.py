import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import tempfile
import unittest
from unittest import mock

spec=importlib.util.spec_from_file_location('private_steer_import',
    Path(__file__).resolve().parents[2]/'.agents/skills/yggterm-agent-fleet/ygg-steer-import.py')
mod=importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

# Invented legacy checkout. Nothing here is real private steering content;
# the assembler is a structurally identical twin of the yggsteer-steer-v1
# mechanism the versioned adapter recognizes.
STEER_SOURCE=r'''import pathlib, re, sys, tomllib
ROOT = pathlib.Path(__file__).resolve().parent
CFG = tomllib.loads((ROOT / 'targets.toml').read_text())
DOOR_GUARD = 'fixture-steer:door'
HARNESS_GUARD = 'fixture-steer:harness'

def guarded(name, body):
    return f'<!-- BEGIN {name} -->\n{body.rstrip()}\n<!-- END {name} -->\n'

def extract_foreign(text, tag='fixture-memory'):
    m = re.search(f'<!-- BEGIN {tag} -->.*?<!-- END {tag} -->\n?', text, re.S)
    return m.group(0) if m else None

def assemble(harness, live_text):
    t = CFG[harness]
    parts = [(ROOT / t['core']).read_text().rstrip()]
    parts.append(guarded('fixture-steer:self', (ROOT / 'sections/self-door.md').read_text()))
    parts.append(guarded('fixture-steer:orders', (ROOT / 'sections/orders.md').read_text()))
    parts.append(guarded(DOOR_GUARD, (ROOT / 'sections/board.md').read_text()))
    if t.get('local'):
        parts.append(guarded(HARNESS_GUARD, (ROOT / t['local']).read_text()))
    stamp = extract_foreign(live_text) if live_text else None
    if stamp:
        parts.append(stamp.rstrip() + '\n')
    else:
        print('fixture warning: foreign slot empty', file=sys.stderr)
    return '\n\n'.join(parts)
'''

TARGETS_SOURCE='''\
[alpha]
target = '~/.fixture-alpha-home/AGENTS.md'
core = 'core/alpha.md'

[beta]
target = '~/.fixture-beta-home/ROOT.md'
core = 'core/beta.md'
local = 'harness/beta.md'

[hosts]
fleet = ['fixture-host-one', 'fixture-host-two']
'''

FIXTURE_FILES={
    'core/alpha.md':'Alpha core policy.\n',
    'core/beta.md':'Beta core policy.\n',
    'sections/self-door.md':'Self door body.\n',
    'sections/orders.md':'Orders body.\n',
    'sections/board.md':'Board body.\n',
    'harness/beta.md':'Beta local harness text.\n',
    'NOTICE':'fixture notice file\n',
    'skills/orphan/SKILL.md':'orphan fixture skill body\n',
    '.git/HEAD':'fixture history reference\n',
}

LIVE_SNAPSHOT=(b'operator preamble\n'
    b'<!-- BEGIN fixture-memory -->\nmemory slot line\n<!-- END fixture-memory -->\n')

# Hand-pinned expected projection for harness beta (not a self-roundtrip):
# literal core, guarded sections in assembly order, local wrap last, then the
# preserved foreign-owner region, all joined with two newlines.
EXPECTED_PROJECTION=(
    'Beta core policy.\n\n'
    '<!-- BEGIN fixture-steer:self -->\nSelf door body.\n<!-- END fixture-steer:self -->\n\n\n'
    '<!-- BEGIN fixture-steer:orders -->\nOrders body.\n<!-- END fixture-steer:orders -->\n\n\n'
    '<!-- BEGIN fixture-steer:door -->\nBoard body.\n<!-- END fixture-steer:door -->\n\n\n'
    '<!-- BEGIN fixture-steer:harness -->\nBeta local harness text.\n<!-- END fixture-steer:harness -->\n\n\n'
    '<!-- BEGIN fixture-memory -->\nmemory slot line\n<!-- END fixture-memory -->\n').encode()


def build_legacy(root):
    for rel,text in FIXTURE_FILES.items():
        path=root/rel;path.parent.mkdir(parents=True,exist_ok=True)
        if path.exists() or path.is_symlink():path.unlink()
        path.write_text(text)
    (root/'steer').write_text(STEER_SOURCE)
    (root/'targets.toml').write_text(TARGETS_SOURCE)


def tree_digest(root):
    rows={}
    for base,dirs,names in os.walk(root):
        for name in dirs+names:
            path=Path(base)/name;entry=path.lstat()
            kind=('dir' if stat.S_ISDIR(entry.st_mode) else
                  'link' if stat.S_ISLNK(entry.st_mode) else
                  'fifo' if stat.S_ISFIFO(entry.st_mode) else 'file')
            digest=''
            if kind=='file':digest=hashlib.sha256(path.read_bytes()).hexdigest()
            rows[str(path.relative_to(root))]=(kind,digest)
    return rows


class Import(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name)
        self.legacy=self.root/'legacy';self.legacy.mkdir()
        build_legacy(self.legacy)
        self.home=self.root/'home';self.home.mkdir(mode=0o700)
        self.env={'XDG_CONFIG_HOME':str(self.home/'config')}
        self.ygg=self.root/'yggroot';self.ygg.mkdir(mode=0o700)
        mod.fs.initialize(home=self.home,env=self.env,explicit=str(self.ygg))

    def plan(self):
        return mod.plan_import(self.legacy)

    def stage(self,plan=None,manifest=None):
        plan=plan or self.plan();manifest=manifest or mod.build_manifest(plan)
        return mod.stage_candidate(plan,manifest,home=self.home,env=self.env,root=str(self.ygg))

    def test_plan_classifies_and_hashes_without_writing(self):
        before=tree_digest(self.legacy)
        plan=self.plan()
        self.assertEqual(plan['adapter']['kind'],'yggsteer-steer-v1')
        self.assertEqual(plan['sections'],[
            {'guard':'fixture-steer:self','source':'sections/self-door.md'},
            {'guard':'fixture-steer:orders','source':'sections/orders.md'},
            {'guard':'fixture-steer:door','source':'sections/board.md'}])
        self.assertEqual(plan['guard_constants'],{'DOOR_GUARD':'fixture-steer:door',
                                                  'HARNESS_GUARD':'fixture-steer:harness'})
        self.assertEqual(plan['foreign']['tag'],'fixture-memory')
        self.assertFalse(plan['foreign']['required'])
        self.assertEqual(plan['sources']['sections/orders.md'],
                         {'sha256':hashlib.sha256(b'Orders body.\n').hexdigest(),
                          'bytes':len(b'Orders body.\n'),'owner':mod._owner_known(os.getuid())})
        self.assertIn('NOTICE',plan['blockers'])
        self.assertIn('skills/orphan/SKILL.md',plan['blockers'])
        self.assertNotIn('.git/HEAD',plan['blockers'])
        self.assertTrue(any(row['path']=='.git' and 'parent' in row['disposition']
                            for row in plan['git_history']))
        self.assertEqual(plan['placement']['observed_keys'],['fleet'])
        self.assertEqual(plan['delete_eligible'],[])
        self.assertFalse(plan['sealed'])
        self.assertEqual(tree_digest(self.legacy),before)

    def test_pinned_known_projection_through_accepted_compiler(self):
        plan=self.plan();manifest=mod.build_manifest(plan)
        beta=manifest['targets']['beta']
        self.assertEqual([p['mode'] for p in beta['parts']],
                         ['literal','guarded','guarded','guarded','guarded'])
        self.assertEqual(beta['parts'][0]['source'],'core/beta.md')
        self.assertEqual(beta['parts'][-1]['guard'],'fixture-steer:harness')
        self.assertEqual(beta['target'],'.fixture-beta-home/ROOT.md')
        self.assertEqual(beta['preserve'],[{'tag':'fixture-memory','required':False}])
        result=mod.compile_preview(self.legacy,manifest,'beta',LIVE_SNAPSHOT)
        self.assertEqual(result['output'],EXPECTED_PROJECTION)
        self.assertEqual(result['authority'],'proposal-only')
        self.assertFalse(result['target_written'])
        alpha=mod.compile_preview(self.legacy,manifest,'alpha')
        self.assertNotIn(b'fixture-steer:harness',alpha['output'])
        self.assertFalse((self.root/'home'/'.fixture-beta-home').exists())

    def test_historical_literal_comments_stay_opaque(self):
        source=self.legacy/'core/beta.md'
        text=('<!-- BEGIN legacy-times:note (historical,\nowner-authored annotation) -->\n'
              'Historical note stays verbatim.\n<!-- END legacy-times:note -->\n')
        source.write_text('Beta core policy.\n'+text)
        plan=self.plan();manifest=mod.build_manifest(plan)
        result=mod.compile_preview(self.legacy,manifest,'beta')
        self.assertTrue(result['output'].startswith(('Beta core policy.\n'+text.rstrip()).encode()))
        self.assertEqual(result['output'].count(b'legacy-times:note'),2)

    def test_foreign_region_required_and_malformed_hold(self):
        plan=self.plan();manifest=mod.build_manifest(plan)
        for text in [b'<!-- BEGIN fixture-memory -->',
                     b'<!-- END fixture-memory --><!-- BEGIN fixture-memory -->',
                     b'<!-- BEGIN fixture-memory -->a<!-- END fixture-memory -->'
                     b'<!-- BEGIN fixture-memory -->b<!-- END fixture-memory -->']:
            with self.subTest(text=text),self.assertRaises(mod.fs.Held):
                mod.compile_preview(self.legacy,manifest,'beta',text)
        required=json.loads(json.dumps(manifest))
        required['targets']['beta']['preserve'][0]['required']=True
        with self.assertRaisesRegex(mod.fs.Held,'absent'):
            mod.compile_preview(self.legacy,required,'beta',b'no region here')

    def test_unsupported_ast_holds_and_bomb_is_never_executed(self):
        bomb=self.legacy/'sections/bomb-marker'
        looped=STEER_SOURCE.replace(
            "    parts.append(guarded('fixture-steer:orders', (ROOT / 'sections/orders.md').read_text()))",
            "    for name in ['orders']:\n"
            "        parts.append(guarded(name, (ROOT / 'sections/orders.md').read_text()))")
        (self.legacy/'steer').write_text(looped)
        with self.assertRaisesRegex(mod.fs.Held,'unsupported'):
            self.plan()
        bombish=STEER_SOURCE.replace(
            "    else:\n        print('fixture warning: foreign slot empty', file=sys.stderr)",
            "    else:\n        open(r'%s','w').write('boom')" % bomb)
        (self.legacy/'steer').write_text(bombish)
        plan=self.plan()
        self.assertEqual(plan['adapter']['kind'],'yggsteer-steer-v1')
        self.assertFalse(bomb.exists())

    def test_unknown_toml_fields_captured_and_staging_refused(self):
        (self.legacy/'targets.toml').write_text(
            TARGETS_SOURCE.replace("[alpha]\n","[alpha]\nmode = 'fast'\n"))
        plan=self.plan()
        self.assertEqual(plan['unknown'],
                         [{'scope':'harness','harness':'alpha','fields':['mode']}])
        with self.assertRaisesRegex(mod.fs.Held,'unknown fields'):
            mod.build_manifest(plan)
        beta=mod.build_manifest(plan,['beta'])
        self.assertIn('beta',beta['targets'])
        with self.assertRaisesRegex(mod.fs.Held,'unknown fields'):
            self.stage(plan,mod.build_manifest(plan))
        self.assertEqual(list((self.ygg/'steer'/'candidates').glob('*')) if (self.ygg/'steer').exists() else [],[])

    def test_paths_symlink_fifo_bounds_and_target_hold(self):
        (self.legacy/'targets.toml').write_text(
            TARGETS_SOURCE.replace("core = 'core/alpha.md'","core = '../outside.md'"))
        with self.assertRaisesRegex(mod.fs.Held,'absent or not plain'):self.plan()
        build_legacy(self.legacy)
        section=self.legacy/'sections/board.md';section.unlink()
        section.symlink_to(self.legacy/'sections/orders.md')
        with self.assertRaises(mod.fs.Held):self.plan()
        build_legacy(self.legacy)
        orders=self.legacy/'sections/orders.md';orders.unlink();os.mkfifo(orders)
        with self.assertRaisesRegex(mod.fs.Held,'absent or not plain'):self.plan()
        build_legacy(self.legacy)
        (self.legacy/'sections/board.md').write_text('x'*(mod.LIMIT_PART_BYTES+1))
        with self.assertRaisesRegex(mod.fs.Held,'bounded'):self.plan()
        build_legacy(self.legacy)
        (self.legacy/'targets.toml').write_text(
            TARGETS_SOURCE.replace("target = '~/.fixture-alpha-home/AGENTS.md'",
                                   "target = '/opt/outside-home/AGENTS.md'"))
        plan=self.plan()
        self.assertTrue(any(row.get('harness')=='alpha' for row in plan['hold_items']))
        self.assertIsNone(plan['harnesses']['alpha']['target_relative'])
        with self.assertRaisesRegex(mod.fs.Held,'hold items'):
            mod.build_manifest(plan,['alpha'])
        with mock.patch.object(mod,'_owner_known',side_effect=mod.fs.Held('unknown owner')):
            with self.assertRaisesRegex(mod.fs.Held,'unknown owner'):self.plan()

    def test_source_mutation_while_read_holds(self):
        original=mod._slurp
        def mutator(stream,limit):
            raw=original(stream,limit)
            with open(self.legacy/'sections/orders.md','ab') as handle:handle.write(b'late append')
            return raw
        with mock.patch.object(mod,'_slurp',mutator):
            with self.assertRaisesRegex(mod.fs.Held,'changed or grew'):self.plan()

    def test_stage_layout_privacy_and_no_other_writes(self):
        decoy_policy=self.ygg/'steer'/'active.json'
        decoy_policy.parent.mkdir(parents=True,exist_ok=True)
        decoy_policy.write_text('{"decoy":true}\n')
        decoy_markdown=self.ygg/'steer'/'policy.md';decoy_markdown.write_text('decoy\n')
        legacy_before=tree_digest(self.legacy)
        root_before=tree_digest(self.ygg)
        plan=self.plan();manifest=mod.build_manifest(plan)
        result=self.stage(plan,manifest)
        self.assertTrue(result['published'])
        self.assertEqual(result['candidate_id'],mod.candidate_identity(plan,manifest))
        base=self.ygg/'steer'/'candidates'/result['candidate_id']
        self.assertEqual(json.loads((base/'manifest.json').read_text()),manifest)
        marker=json.loads((base/'complete.json').read_text())
        self.assertEqual(marker['candidate_id'],result['candidate_id'])
        self.assertTrue(marker['sealed'])
        for rel,digest in marker['files'].items():
            self.assertEqual(hashlib.sha256((base/rel).read_bytes()).hexdigest(),digest)
        for path in base.rglob('*'):
            if path.is_dir():self.assertEqual(path.stat().st_mode&0o777,0o700,path)
            else:self.assertEqual(path.stat().st_mode&0o777,0o600,path)
        self.assertEqual((base/'provenance.json').stat().st_mode&0o777,0o600)
        self.assertEqual(decoy_policy.read_text(),'{"decoy":true}\n')
        self.assertEqual(decoy_markdown.read_text(),'decoy\n')
        after=tree_digest(self.ygg)
        rel_base=str(base.relative_to(self.ygg))
        new=set(after)-set(root_before)
        expected_new={'steer/candidates',rel_base}|{
            str(p.relative_to(self.ygg)) for p in base.rglob('*')}
        self.assertEqual(new,expected_new)
        self.assertEqual({k:v for k,v in after.items() if k in root_before},root_before)
        self.assertEqual(tree_digest(self.legacy),legacy_before)
        self.assertFalse((self.home/'.fixture-beta-home').exists())

    def test_partial_failure_publishes_nothing(self):
        plan=self.plan();manifest=mod.build_manifest(plan)
        candidate_id=mod.candidate_identity(plan,manifest)
        original=mod._Staging.put
        def flaky(self_staging,relative,data):
            if relative.startswith('sources/'):raise OSError(28,'simulated full disk')
            return original(self_staging,relative,data)
        with mock.patch.object(mod._Staging,'put',flaky):
            with self.assertRaisesRegex(OSError,'full disk'):self.stage(plan,manifest)
        candidates=self.ygg/'steer'/'candidates'
        self.assertFalse((candidates/candidate_id).exists())
        self.assertEqual([p for p in candidates.iterdir() if not p.name.startswith('.')],[])
        self.assertEqual([p for p in candidates.iterdir() if p.name.startswith('.staging-')],[])
        result=self.stage(plan,manifest)
        self.assertTrue(result['published'])

    def test_replay_idempotent_and_occupied_candidate_holds(self):
        first=self.stage()
        manifest_path=Path(first['path'])/'manifest.json'
        inode=manifest_path.stat().st_ino
        second=self.stage()
        self.assertFalse(second['published'])
        self.assertTrue(second['validated'])
        self.assertEqual(second['candidate_id'],first['candidate_id'])
        self.assertEqual(manifest_path.stat().st_ino,inode)
        occupied=self.ygg/'steer'/'candidates'/first['candidate_id']
        (occupied/'complete.json').write_text(json.dumps(
            {'schema':1,'candidate_id':'other','sealed':True,'files':{}}))
        with self.assertRaisesRegex(mod.fs.Held,'identity disagrees'):self.stage()
        self.assertEqual(json.loads((occupied/'complete.json').read_text())['candidate_id'],'other')
        (occupied/'complete.json').unlink()
        with self.assertRaisesRegex(mod.fs.Held,'unreadable'):self.stage()
        (occupied/'complete.json').write_text(json.dumps(
            {'schema':1,'candidate_id':first['candidate_id'],'sealed':True,
             'manifest_sha256':'0'*64,'files':{}}))
        with self.assertRaisesRegex(mod.fs.Held,'sealed file list'):self.stage()

    def test_source_change_between_plan_and_stage(self):
        plan=self.plan();manifest=mod.build_manifest(plan)
        (self.legacy/'sections/orders.md').write_text('Changed orders body.\n')
        with self.assertRaisesRegex(mod.fs.Held,'changed since the plan'):self.stage(plan,manifest)
        replan=self.plan()
        self.assertNotEqual(mod.candidate_identity(replan,mod.build_manifest(replan)),
                            mod.candidate_identity(plan,manifest))

    def test_path_independent_identity(self):
        plan=self.plan();manifest=mod.build_manifest(plan)
        twin=self.root/'twin-legacy';shutil.copytree(self.legacy,twin)
        twin_plan=mod.plan_import(twin)
        self.assertEqual(mod.candidate_identity(plan,manifest),
                         mod.candidate_identity(twin_plan,mod.build_manifest(twin_plan)))
        self.assertEqual(plan['sources'],twin_plan['sources'])

    def test_unclassified_files_preserved_never_staged(self):
        os.mkfifo(self.legacy/'fifo-pipe')
        plan=self.plan()
        kinds={row['path']:row['kind'] for row in plan['unclassified']}
        self.assertEqual(kinds.get('fifo-pipe'),'fifo')
        self.assertIn('skills/orphan/SKILL.md',plan['blockers'])
        result=self.stage(plan,mod.build_manifest(plan))
        sources=(Path(result['path'])/'sources')
        staged={str(p.relative_to(sources)) for p in sources.rglob('*') if p.is_file()}
        declared=set(plan['sources'])-{'steer','targets.toml'}
        self.assertEqual(staged,declared)
        self.assertTrue((self.legacy/'skills/orphan/SKILL.md').exists())
        self.assertTrue(self.legacy.joinpath('fifo-pipe').exists())

    def test_stage_requires_initialized_root(self):
        plan=self.plan();manifest=mod.build_manifest(plan)
        uninitialized=self.root/'uninitialized';uninitialized.mkdir(mode=0o700)
        with self.assertRaisesRegex(mod.fs.Held,'initialize the root identity'):
            mod.stage_candidate(plan,manifest,home=self.home,env=self.env,
                                root=str(uninitialized))
        naked=dict(plan);naked.pop('legacy_root_resolved')
        with self.assertRaisesRegex(mod.fs.Held,'resolved legacy root'):
            mod.stage_candidate(naked,manifest,home=self.home,env=self.env,root=str(self.ygg))

    def test_cli_plan_reports_hold_without_writing(self):
        before=tree_digest(self.legacy)
        from subprocess import run
        result=run(['python3',str(Path(mod.__file__)),'--legacy',str(self.legacy),'plan'],
                   capture_output=True,text=True)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertIn('blockers preserved in place',result.stdout)
        self.assertEqual(tree_digest(self.legacy),before)
        broken=self.root/'broken-legacy';broken.mkdir();broken.joinpath('targets.toml').write_text('[a]\n')
        held=run(['python3',str(Path(mod.__file__)),'--legacy',str(broken),'plan'],
                 capture_output=True,text=True)
        self.assertEqual(held.returncode,75)
        self.assertIn('HOLD',held.stderr)

    def test_real_legacy_checkout_smoke(self):
        real=Path.home()/'git'/'yggsteer'
        if not (real/'steer').is_file():
            self.skipTest('real legacy checkout not present')
        plan=mod.plan_import(real)
        self.assertEqual(plan['adapter']['kind'],'yggsteer-steer-v1')
        self.assertEqual(len(plan['sections']),16)
        self.assertEqual(plan['sections'][0]['guard'],'yggterm-steer:self')
        self.assertEqual(plan['guard_constants']['DOOR_GUARD'],'yggterm-steer:msgboard')
        self.assertEqual(plan['guard_constants']['HARNESS_GUARD'],'yggterm-steer:harness')
        self.assertEqual(plan['foreign']['tag'],'yggterm-memory')
        self.assertTrue(plan['blockers'])
        for rel in plan['blockers']:
            self.assertTrue((real/rel).exists() or (real/rel).is_symlink(),rel)
        manifest=mod.build_manifest(plan)
        for name,target in manifest['targets'].items():
            self.assertTrue(target['target'].startswith('.'))
            self.assertEqual(target['preserve'],[{'tag':'yggterm-memory','required':False}])


if __name__=='__main__':unittest.main()
