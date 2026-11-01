"""Hermetic read-only policy preflight; invented activated state only."""
from copy import deepcopy
from dataclasses import asdict, FrozenInstanceError, replace
import json
import os
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest
from unittest.mock import patch
import uuid

from . import swarmconfig as sc
from . import review_policy as rp
from .review_completion import Admission, binding_digest


def fixture():
    # Existing public conformance seed, qualified with invented references.
    cfg, fault = sc.parse_yaml_bytes((Path(__file__).parent /
        'fixtures/valid/genesis.yaml').read_bytes())
    assert fault is None
    cfg['fleet'] = dict(authority_host='example-host', dispatch_host='example-host',
                        hosts=['example-host', 'other-host'])
    cfg['maintenance']['supervisor']['controller_host'] = 'example-host'
    cat = cfg['catalog']
    cat['harnesses']['h1'] = dict(kind='consult-cli', adapter_ref='consult-codex-v1')
    cat['providers']['p1'] = dict(id='openai')
    cat['accounts']['a1']['permitted_hosts'] = ['example-host']
    cat['plans']['pl1']['account'] = 'a1'
    cat['bindings']['b1']['reasoning'] = 'high'
    cat['bindings']['b1']['capabilities'] = ['editorial', 'review']
    cfg['roles']['editor'].update(scenario_policy_ref='swap', authority_ref='example-owner',
                                required_capabilities=['editorial'])
    cfg['editor_scenarios']['swap']['activation']['authority_ref'] = 'example-owner'
    return cfg


class PolicyTest(unittest.TestCase):
    def setUp(self):
        scratch = Path.home() / '.yggterm/scratchpad/review-policy-tests'
        scratch.mkdir(parents=True, exist_ok=True)
        self.tmp = tempfile.TemporaryDirectory(dir=scratch)
        self.root = Path(self.tmp.name)
        self.state = self.root / 'state'
        self.live = self.root / 'unused.yaml'
        self.cfg = fixture()
        self.activate(self.cfg)

    def tearDown(self):
        self.tmp.cleanup()

    def activate(self, cfg):
        receipt = sc.accept(cfg, proposed_by='owner', state_dir=self.state,
                            change_id=cfg['identity']['change_id'])
        self.assertEqual(receipt['decision'], 'accepted', receipt)

    def observe(self):
        return rp.observe_default_editor(self.state, self.live, 'example-host')

    def loaded(self, cfg=None):
        cfg = deepcopy(self.cfg if cfg is None else cfg)
        return dict(mode='active', config=cfg, source='release', faults=[], denials={},
            loaded_at='example-observation', revision=dict(epoch=1, revision=1,
                content_hash=sc.content_hash(cfg), change_id=cfg['identity']['change_id']))

    def held(self, loaded):
        with patch.object(sc, 'load', return_value=loaded):
            with self.assertRaises(rp.Hold):
                self.observe()

    def admission(self, selection):
        return Admission(1, 'example-task', str(uuid.uuid4()), '1' * 64,
            ('Original source criterion',), 'example-worker', str(uuid.uuid4()),
            selection.editor_identity, selection.editor_binding_digest,
            selection.config_digest, 0)

    def test_real_activation_selection_and_digests(self):
        before = {str(p.relative_to(self.state)): p.read_bytes()
                  for p in self.state.rglob('*') if p.is_file()}
        s = self.observe()
        self.assertEqual(s.editor_identity, 'b1')
        self.assertEqual(s.editor_binding, dict(identity='b1', **self.cfg['catalog']['bindings']['b1']))
        self.assertEqual(s.editor_binding_digest, binding_digest(s.editor_binding))
        self.assertEqual(s.config_digest, sc.content_hash(self.cfg))
        self.assertEqual(s.policy_digest, binding_digest(s.policy))
        self.assertEqual((s.account_ref, s.plan_ref, s.authority_ref),
                         ('a1', 'pl1', 'example-owner'))
        self.assertEqual(before, {str(p.relative_to(self.state)): p.read_bytes()
                  for p in self.state.rglob('*') if p.is_file()})

    def test_immutable_and_no_loader_or_returned_alias(self):
        loaded = self.loaded()
        with patch.object(sc, 'load', return_value=loaded):
            s = self.observe()
        loaded['config']['catalog']['bindings']['b1']['capabilities'].append('changed')
        b = s.editor_binding
        b['capabilities'].append('changed-again')
        p = s.policy
        p['role']['required_capabilities'].clear()
        r = s.revision
        r.clear()
        self.assertEqual(s.editor_binding['capabilities'], ['editorial', 'review'])
        self.assertEqual(s.policy['role']['required_capabilities'], ['editorial'])
        self.assertEqual(s.revision['revision'], 1)
        with self.assertRaises(FrozenInstanceError):
            s.host_ref = 'other-host'

    def test_admission_exact_and_unchanged(self):
        a = self.admission(self.observe())
        before = asdict(a)
        self.assertEqual(rp.check_admitted_editor(a, self.state, self.live,
                         'example-host'), self.observe())
        self.assertEqual(asdict(a), before)
        for field, value in [('editor_identity', 'other'),
                             ('editor_binding_digest', '2' * 64),
                             ('config_digest', '3' * 64)]:
            with self.subTest(field=field), self.assertRaises(rp.Hold):
                rp.check_admitted_editor(replace(a, **{field: value}),
                                        self.state, self.live, 'example-host')
        with self.assertRaises(rp.Hold):
            rp.check_admitted_editor({}, self.state, self.live, 'example-host')

    def test_fresh_config_and_binding_drift(self):
        a = self.admission(self.observe())
        cfg = deepcopy(self.cfg)
        cfg['identity'].update(revision=2, parent_hash=sc.content_hash(self.cfg),
                               change_id=str(uuid.uuid4()))
        cfg['catalog']['bindings']['b1']['model'] = 'invented-new-model'
        self.activate(cfg)
        with self.assertRaises(rp.Hold):
            rp.check_admitted_editor(a, self.state, self.live, 'example-host')

    def test_invalid_and_unqualified_policy_matrix(self):
        changes = [
            (('catalog', 'bindings', 'b1', 'status'), 'declared'),
            (('catalog', 'bindings', 'b1', 'status'), 'disabled'),
            (('catalog', 'bindings', 'b1', 'account'), 'missing'),
            (('catalog', 'bindings', 'b1', 'plan'), 'missing'),
            (('catalog', 'bindings', 'b1', 'provider'), 'missing'),
            (('catalog', 'bindings', 'b1', 'harness'), 'missing'),
            (('catalog', 'accounts', 'a1', 'provider'), 'missing'),
            (('catalog', 'plans', 'pl1', 'provider'), 'missing'),
            (('catalog', 'plans', 'pl1', 'account'), 'missing'),
            (('catalog', 'accounts', 'a1', 'permitted_hosts'), ['other-host']),
            (('catalog', 'accounts', 'a1', 'permitted_hosts'), None),
            (('catalog', 'bindings', 'b1', 'capabilities'), []),
            (('catalog', 'bindings', 'b1', 'capabilities'), ['editorial']),
            (('catalog', 'bindings', 'b1', 'capabilities'), ['review']),
            (('catalog', 'bindings', 'b1', 'task_restrictions'), ['unknown-task']),
            (('catalog', 'bindings', 'b1', 'task_restrictions'), False),
            (('catalog', 'bindings', 'b1', 'reasoning'), 'unsupported'),
            (('catalog', 'bindings', 'b1', 'model'), ''),
            (('catalog', 'providers', 'p1', 'id'), 'unsupported'),
            (('catalog', 'harnesses', 'h1', 'adapter_ref'), 'unknown-adapter'),
            (('roles', 'editor', 'kind'), 'execution'),
            (('roles', 'editor', 'authority_ref'), ''),
            (('roles', 'editor', 'scenario_policy_ref'), 'missing'),
            (('roles', 'editor', 'default_binding_ref'), 'missing'),
            (('roles', 'editor', 'required_capabilities'), ['missing']),
            (('schema_version',), 999),
            (('catalog', 'unexpected'), {}),
        ]
        for path, value in changes:
            with self.subTest(path=path, value=value):
                cfg = deepcopy(self.cfg)
                parent = cfg
                for key in path[:-1]:
                    parent = parent[key]
                parent[path[-1]] = value
                self.held(self.loaded(cfg))
        # Valid catalog references with incompatible relationships also HOLD.
        for target, value in [('provider', 'p2'), ('account', 'a2')]:
            cfg = deepcopy(self.cfg)
            cfg['catalog']['providers']['p2'] = dict(id='invented-provider')
            cfg['catalog']['accounts']['a2'] = dict(provider='p1')
            cfg['catalog']['plans']['pl1'][target] = value
            self.held(self.loaded(cfg))

    def test_missing_role_and_valid_account_provider_mismatch(self):
        cfg = deepcopy(self.cfg)
        del cfg['roles']['editor']
        self.held(self.loaded(cfg))
        cfg = deepcopy(self.cfg)
        cfg['catalog']['providers']['p2'] = dict(id='invented-provider')
        cfg['catalog']['accounts']['a1']['provider'] = 'p2'
        self.held(self.loaded(cfg))
        with self.assertRaises(rp.Hold):
            rp.observe_default_editor(self.state, self.live, 'guessed-host')

    def test_collection_pointer_selects_default_without_triggering_fallback(self):
        cfg = deepcopy(self.cfg)
        cfg['roles']['editor']['scenario_policy_ref'] = 'editor_scenarios'
        # A fallback may require a different authority and need not list the
        # already admitted default as its target. Observing it executes none.
        cfg['catalog']['bindings']['fallback'] = deepcopy(cfg['catalog']['bindings']['b1'])
        cfg['editor_scenarios']['swap']['activation']['authority_ref'] = 'another-authority'
        cfg['editor_scenarios']['swap']['binding_order'] = ['fallback']
        with patch.object(sc, 'load', return_value=self.loaded(cfg)):
            selected = self.observe()
        self.assertEqual(selected.editor_identity, 'b1')
        self.assertEqual(selected.authority_ref, 'example-owner')
        self.assertEqual(selected.scenario_policy_ref, 'editor_scenarios')
        self.assertEqual(selected.policy['scenarios'], cfg['editor_scenarios'])
        self.assertFalse((self.state / 'editorial/current-editor.json').exists())

    def test_named_scenario_is_preserved_as_metadata_without_activation(self):
        cfg = deepcopy(self.cfg)
        cfg['editor_scenarios']['swap']['activation']['authority_ref'] = 'another-authority'
        with patch.object(sc, 'load', return_value=self.loaded(cfg)):
            selected = self.observe()
        self.assertEqual(selected.policy['scenarios']['swap']['activation']['authority_ref'],
                         'another-authority')
        self.assertEqual(selected.authority_ref, 'example-owner')

    def test_verified_checkpoint_and_revision_disagreement(self):
        release = self.state / 'config/releases' / sc.content_hash(self.cfg) / 'release.json'
        release.write_text('corrupt')
        self.assertEqual(sc.load(self.state)['source'], 'checkpoint')
        self.assertEqual(self.observe().config_digest, sc.content_hash(self.cfg))
        head = self.state / 'config/active.json'
        body = json.loads(head.read_text())
        body['revision'] = 2
        head.write_text(json.dumps(body))
        with self.assertRaises(rp.Hold):
            self.observe()

    def test_load_modes_faults_shapes_and_revision(self):
        for mode in ['last_good', 'containment_only', 'faulted', 'unknown', None]:
            value = self.loaded()
            value['mode'] = mode
            self.held(value)
        for value in [None, {}, [], dict(self.loaded(), extra=True),
                      dict(self.loaded(), config={}), dict(self.loaded(), revision={}),
                      dict(self.loaded(), faults=[{'code': 'fault'}]),
                      dict(self.loaded(), denials=None),
                      dict(self.loaded(), denials={'unknown': {}}),
                      dict(self.loaded(), revision=dict(self.loaded()['revision'], epoch=True))]:
            self.held(value)
        with patch.object(sc, 'load', side_effect=OSError('invented failure')):
            with self.assertRaises(rp.Hold):
                self.observe()

    def test_real_unavailable_and_last_good_never_empty_healthy(self):
        self.assertEqual(sc.load(self.state)['mode'], 'active')
        (self.state / 'config/active.json').write_text('corrupt')
        self.assertEqual(sc.load(self.state)['mode'], 'containment_only')
        with self.assertRaises(rp.Hold):
            self.observe()
        journal = self.state / 'ledger/config/journal.jsonl'
        journal.parent.mkdir(parents=True)
        journal.write_text(json.dumps(dict(canonical_hash=sc.content_hash(self.cfg)))+'\n')
        self.assertEqual(sc.load(self.state)['mode'], 'last_good')
        with self.assertRaises(rp.Hold):
            self.observe()

    def test_controls_override_corruption_and_storage(self):
        control = self.state / 'control/deny.json'
        control.parent.mkdir(parents=True, exist_ok=True)
        for raw in ['broken', '{}', '[]', '{"latches": []}',
                    '{"latches":{"stop":{}},"latches":{},"updated_at":"example"}',
                    json.dumps(dict(latches={'example-task': {}}, updated_at='example')),
                    json.dumps(dict(latches={}, updated_at='example', unknown=True))]:
            control.write_text(raw)
            with self.subTest(raw=raw), self.assertRaises(rp.Hold):
                self.observe()
        control.write_text(json.dumps(dict(latches={}, updated_at='example')))
        self.assertNotEqual(self.observe().control_digest,
                            binding_digest(dict(deny=None, override='absent')))
        override = self.state / 'editorial/current-editor.json'
        override.parent.mkdir()
        for raw in ['broken', '{}', '{"expires_at":"old"}']:
            override.write_text(raw)
            with self.assertRaises(rp.Hold) as caught:
                self.observe()
            self.assertEqual(caught.exception.code, 'unqualified_editor_override')
        override.unlink()
        override.symlink_to(self.root / 'absent')
        with self.assertRaises(rp.Hold):
            self.observe()
        override.unlink()
        with patch.object(sc, '_read_storage_text', side_effect=PermissionError('invented')):
            with self.assertRaises(rp.Hold):
                self.observe()

    def test_actual_fifo_policy_leaves_hold_promptly(self):
        for relative in ['control/deny.json', 'config/active.json',
                         'config/checkpoint.json', 'ledger/config/journal.jsonl']:
            with self.subTest(relative=relative):
                target = self.state / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                original = target.read_bytes() if target.is_file() else None
                if target.exists():
                    target.unlink()
                os.mkfifo(target)
                # Force the canonical recovery paths as well as primary reads.
                head = self.state / 'config/active.json'
                head_original = head.read_bytes() if relative != 'config/active.json' else None
                release = self.state / 'config/releases' / sc.content_hash(self.cfg) / 'release.json'
                release_original = release.read_bytes()
                if relative == 'config/checkpoint.json':
                    release.write_text('corrupt')
                if relative == 'ledger/config/journal.jsonl':
                    head.write_text('corrupt')
                command = [sys.executable, '-c',
                    'from swarmharness import review_policy as p\n'
                    'try:p.observe_default_editor(' + repr(str(self.state)) + ','
                    + repr(str(self.live)) + ',"example-host")\n'
                    'except p.Hold:print("HOLD")\n'
                    'else:raise SystemExit("unexpected selection")\n']
                try:
                    environment = dict(os.environ, PYTHONPATH=str(Path(__file__).resolve().parents[1]))
                    result = subprocess.run(command, env=environment, capture_output=True,
                                            text=True, timeout=3)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stdout.strip(), 'HOLD')
                finally:
                    target.unlink()
                    if original is not None:
                        target.write_bytes(original)
                    if head_original is not None:
                        head.write_bytes(head_original)
                    release.write_bytes(release_original)

    def test_fifo_release_and_oversized_reader_are_bounded(self):
        release = self.state / 'config/releases' / sc.content_hash(self.cfg) / 'release.json'
        release.unlink()
        os.mkfifo(release)
        # A verified regular checkpoint can still supply the active config.
        selected = self.observe()
        self.assertEqual(selected.config_digest, sc.content_hash(self.cfg))
        large = self.root / 'oversized'
        large.write_bytes(b'x' * 33)
        with self.assertRaises(ValueError):
            sc._read_storage_text(large, 32)


if __name__ == '__main__':
    unittest.main()
