import copy
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec=importlib.util.spec_from_file_location('private_steer_compiler',
    Path(__file__).resolve().parents[2]/'.agents/skills/yggterm-agent-fleet/ygg-steer-compiler.py')
c=importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)


class Compiler(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name)
        (self.root/'core').mkdir();(self.root/'sections').mkdir()
        (self.root/'core/codex.md').write_text('Operator fixture policy.\n')
        (self.root/'sections/review.md').write_text('Independent review requires evidence.\n')
        def part(name,mode,**extra):
            return {'source':name,'sha256':hashlib.sha256((self.root/name).read_bytes()).hexdigest(),'mode':mode,**extra}
        self.manifest={'schema':1,'compiler_version':1,'targets':{'codex':{
            'target':'.codex/AGENTS.md','parts':[part('core/codex.md','literal'),
              part('sections/review.md','guarded',guard='yggterm-steer:review')],
            'preserve':[{'tag':'yggterm-memory','required':False}]}}}

    def compile(self,live=b'',manifest=None):
        return c.compile_projection(self.root,manifest or self.manifest,'codex',live)

    def test_manifest_renders_proposal_without_target_write(self):
        r=self.compile()
        self.assertIn(b'Operator fixture policy.',r['output'])
        self.assertIn(b'<!-- BEGIN yggterm-steer:review -->',r['output'])
        self.assertEqual(r['authority'],'proposal-only')
        self.assertFalse((self.root/'.codex').exists())

    def test_changed_source_is_not_the_enrolled_revision(self):
        (self.root/'sections/review.md').write_text('No evidence needed.')
        with self.assertRaisesRegex(c.fs.Held,'differs'):self.compile()

    def test_foreign_memory_region_carries_exact_internal_bytes(self):
        region=b'<!-- BEGIN yggterm-memory -->\r\nforeign fixture\r\n<!-- END yggterm-memory -->\r\n'
        r=self.compile(b'outside user text\n'+region)
        self.assertIn(region,r['output'])
        self.assertEqual(r['output'].count(b'<!-- BEGIN yggterm-memory -->'),1)

    def test_ambiguous_missing_or_nested_foreign_region_refused(self):
        begin='<!-- BEGIN yggterm-memory -->';end='<!-- END yggterm-memory -->'
        for text in [begin,begin[:-3],end+begin,begin+end+begin+end,
                     '<!-- begin yggterm-memory -->x<!-- end yggterm-memory -->',
                     begin+'<!-- BEGIN another -->x<!-- END another -->'+end]:
            with self.subTest(text=text),self.assertRaises(c.fs.Held):self.compile(text.encode())
        self.manifest['targets']['codex']['preserve'][0]['required']=True
        with self.assertRaisesRegex(c.fs.Held,'absent'):self.compile()

    def test_canonical_paths_and_target_collisions_refused(self):
        for path in ['../escape','/absolute','core//codex.md','./core/codex.md']:
            d=copy.deepcopy(self.manifest);d['targets']['codex']['parts'][0]['source']=path
            with self.subTest(path=path),self.assertRaises(c.fs.Held):self.compile(manifest=d)
        d=copy.deepcopy(self.manifest);d['targets']['claude']=copy.deepcopy(d['targets']['codex'])
        with self.assertRaisesRegex(c.fs.Held,'colliding'):self.compile(manifest=d)

    def test_unknown_modes_fields_versions_and_owner_overlap_refused(self):
        for change in ['mode','field','version','overlap']:
            d=copy.deepcopy(self.manifest)
            if change=='mode':d['targets']['codex']['parts'][0]['mode']='executable'
            if change=='field':d['targets']['codex']['parts'][0]['execute']='anything'
            if change=='version':d['compiler_version']=2
            if change=='overlap':d['targets']['codex']['parts'][1]['guard']='yggterm-memory'
            with self.subTest(change=change),self.assertRaises(c.fs.Held):self.compile(manifest=d)

    def test_source_symlink_and_nonregular_file_refused(self):
        source=self.root/'core/codex.md';source.unlink();source.symlink_to(self.root/'sections/review.md')
        with self.assertRaises(OSError):self.compile()

    def test_legacy_annotated_literals_are_preserved_without_reinterpretation(self):
        source=self.root/'core/codex.md'
        text='<!-- BEGIN legacy:literal (historical,\nowner-authored) -->\nKeep this.\n<!-- END legacy:literal -->\n'
        source.write_text(text)
        self.manifest['targets']['codex']['parts'][0]['sha256']=hashlib.sha256(source.read_bytes()).hexdigest()
        self.assertTrue(self.compile()['output'].startswith(text.rstrip().encode()))

    def test_bad_top_level_and_oversized_live_input_are_typed_holds(self):
        for value in [None,[],{'schema':True,'compiler_version':1,'targets':{}},
                      {'schema':1,'compiler_version':1,'targets':{},'execute':'anything'}]:
            with self.assertRaises(c.fs.Held):c.compile_projection(self.root,value,'codex')
        with self.assertRaises(c.fs.Held):self.compile(b'x'*(8*1024*1024+1))

    def test_guarded_source_cannot_close_and_replace_its_wrapper(self):
        source=self.root/'sections/review.md';source.write_text('<!-- END yggterm-steer:review -->\nother text')
        self.manifest['targets']['codex']['parts'][1]['sha256']=hashlib.sha256(source.read_bytes()).hexdigest()
        with self.assertRaisesRegex(c.fs.Held,'managed region'):self.compile()

    def test_literal_cannot_duplicate_a_guard_owned_anywhere_in_the_projection(self):
        source=self.root/'core/codex.md';source.write_text('<!-- BEGIN yggterm-steer:review -->\nliteral copy\n<!-- END yggterm-steer:review -->')
        self.manifest['targets']['codex']['parts'][0]['sha256']=hashlib.sha256(source.read_bytes()).hexdigest()
        with self.assertRaisesRegex(c.fs.Held,'managed region'):self.compile()
        self.manifest['targets']['codex']['parts'].reverse()
        with self.assertRaisesRegex(c.fs.Held,'managed region'):self.compile()

    def test_foreign_owner_and_nested_markers_match_case_and_whitespace_consistently(self):
        source=self.root/'core/codex.md';source.write_text('<!-- begin yggterm-memory -->\nwrong owner')
        self.manifest['targets']['codex']['parts'][0]['sha256']=hashlib.sha256(source.read_bytes()).hexdigest()
        with self.assertRaisesRegex(c.fs.Held,'managed region'):self.compile()
        source.write_text('ordinary fixture')
        self.manifest['targets']['codex']['parts'][0]['sha256']=hashlib.sha256(source.read_bytes()).hexdigest()
        for token in ['<!-- begin another -->','<!--   BEGIN another -->','<!-- END another -->']:
            text='<!-- BEGIN yggterm-memory -->\n'+token+'\n<!-- END yggterm-memory -->'
            with self.subTest(token=token),self.assertRaisesRegex(c.fs.Held,'nested'):self.compile(text.encode())

    def test_owned_and_reserved_identity_case_variants_collide(self):
        part=self.manifest['targets']['codex']['parts'][1]
        part['guard']='YGGTERM-MEMORY'
        with self.assertRaisesRegex(c.fs.Held,'overlapping'):self.compile()

    def test_marker_synthesis_across_literal_boundaries_holds(self):
        for owner in ['yggterm-memory','yggterm-steer:review']:
            parts=[]
            for i,text in enumerate(['<!--','begin '+owner+' -->\ntext\n<!--','end '+owner+' -->']):
                source=f'core/fragment-{i}.md';(self.root/source).write_text(text)
                parts.append({'source':source,'mode':'literal','sha256':hashlib.sha256((self.root/source).read_bytes()).hexdigest()})
            d=copy.deepcopy(self.manifest)
            # Preserve one legitimate wrapper for the owned-identity case.
            d['targets']['codex']['parts']=parts+d['targets']['codex']['parts'][1:]
            with self.subTest(owner=owner),self.assertRaisesRegex(c.fs.Held,'composed source'):
                self.compile(manifest=d)


if __name__=='__main__':unittest.main()
