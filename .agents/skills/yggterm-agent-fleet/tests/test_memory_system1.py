"""Search falsifiers: no evidence loss, safe failure, bounded private transport."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('memory_system1', Path(__file__).parents[1] / 'ygg-memory.py')
memory = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(memory)


class System1Tests(unittest.TestCase):
    def setUp(self):
        scratch = Path.home() / '.yggterm' / 'scratchpad'
        scratch.mkdir(parents=True, exist_ok=True)
        self.tmp = tempfile.TemporaryDirectory(dir=scratch)
        self.home = Path(self.tmp.name)
        self.env = patch.dict(os.environ, {'YGGTERM_HOME': str(self.home),
            'YGG_SYSTEM1_SESSION_ID': 'synthetic-session', 'JEV_API_KEY': 'synthetic-key'})
        self.env.start()
        self.cfg = dict(memory.SYSTEM1_DEFAULTS, enabled=True)
        self.hits = [('source-a:1', 'peripheral'), ('source-b:2', 'ambiguous'),
                     ('source-c:3', 'direct'), ('source-d:4', 'supporting')]

    def tearDown(self):
        self.env.stop()
        self.tmp.cleanup()

    def config(self, **changes):
        config = dict(self.cfg, **changes)
        path = self.home / 'config' / 'system1.json'
        path.parent.mkdir(exist_ok=True)
        path.write_text(json.dumps(config))

    def test_defaults_and_invalid_config(self):
        self.assertFalse(memory.system1_config()['enabled'])
        for changes in ({'enabled': 'yes'}, {'rate_rps': 3}, {'rate_rps': float('nan')},
                        {'budget_usd_per_session': 1}, {'model': 'unpriced'},
                        {'endpoint': 'http://example.test'}, {'fallback': 'drop'}):
            self.config(**changes)
            with self.assertRaises(ValueError):
                memory.system1_config()

    def test_lookup_and_disabled_have_zero_model_calls(self):
        with patch.object(memory, 'system1_labels', side_effect=AssertionError('model called')):
            self.assertEqual(memory.system1_rerank('orchard irrigation', self.hits)[0], self.hits)
            self.config()
            for query in ('T-101', 'example.md', 'foo.*bar', 'cache', 'hash_123', '.*'):
                self.assertEqual(memory.system1_rerank(query, self.hits)[0], self.hits)

    def test_planted_true_positives_survive_every_label_permutation(self):
        import itertools
        self.config()
        for labels in itertools.product(('direct', 'supporting', 'unrelated', 'unknown'), repeat=4):
            with patch.object(memory, 'system1_labels', return_value=labels):
                ranked, _ = memory.system1_rerank('orchard irrigation', self.hits)
            self.assertCountEqual(ranked, self.hits)
            for i, label in enumerate(labels):
                if label == 'unknown':
                    self.assertEqual(ranked[i], self.hits[i])
        with patch.object(memory, 'system1_labels', return_value=['unrelated','unknown','direct','supporting']):
            ranked, _ = memory.system1_rerank('orchard irrigation', self.hits)
        self.assertEqual(ranked, [self.hits[2], self.hits[1], self.hits[3], self.hits[0]])

    def test_failure_and_malformed_labels_preserve_base(self):
        self.config()
        for result in ([], ['bad'] * 4, ['unknown'] * 4):
            with patch.object(memory, 'system1_labels', return_value=result):
                self.assertEqual(memory.system1_rerank('orchard irrigation', self.hits)[0], self.hits)
        with patch.object(memory, 'system1_labels', side_effect=TimeoutError):
            self.assertEqual(memory.system1_rerank('orchard irrigation', self.hits)[0], self.hits)

    def test_privacy_absence_and_candidates_hold_before_transport(self):
        with patch.dict(os.environ, {}, clear=True):
            with self.assertRaises(ValueError):
                memory.system1_mask('synthetic')
        gate = self.home / 'privacy.py'
        gate.write_text('def l1_scan(text): return [{"span":[0,1]}]\ndef l1_mask(text, findings): return "masked"\n')
        with patch.dict(os.environ, {'YGG_SYSTEM1_PRIVACY_GATE': str(gate)}):
            with patch('urllib.request.build_opener', side_effect=AssertionError('egress')):
                with self.assertRaises(ValueError):
                    memory.system1_labels('orchard irrigation', ['synthetic'], self.cfg)

    def response(self, answers=None, usage=100):
        def answer(choice):
            return {'choice': choice, 'confidence': 0.9,
                    'probabilities': {x: (0.97 if x == choice else 0.01)
                        for x in ('direct','supporting','unrelated','unknown')}}
        result = {'answers': answers or {'0': answer('direct'), '1': answer('supporting')},
                  'usage': {'input_tokens': usage}}
        stream = io.StringIO(json.dumps(result))
        return stream

    def test_batch_cache_counter_and_status(self):
        with patch.object(memory, 'system1_mask', side_effect=lambda x:x), \
             patch('urllib.request.build_opener') as transport:
            transport.return_value.open.side_effect = lambda *a, **kw:self.response()
            self.assertEqual(memory.system1_labels('orchard irrigation', ['a', 'b'], self.cfg), ['direct','supporting'])
            self.assertEqual(memory.system1_labels('orchard irrigation', ['a', 'b'], self.cfg), ['direct','supporting'])
            self.assertEqual(transport.return_value.open.call_count, 1)
        ledger = json.loads((self.home / 'system1' / 'usage.json').read_text())
        self.assertEqual(ledger['events'][0]['input_tokens'],100)
        self.assertEqual(ledger['events'][0]['outcome'],'classified')
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            memory.cmd_system1(None)
        self.assertEqual(json.loads(out.getvalue())['today']['attempts'],1)

    def test_budget_shared_across_calls_and_transport_failure_charged(self):
        self.cfg['budget_usd_per_session'] = 0.00000001
        with patch.object(memory, 'system1_mask', side_effect=lambda x:x), \
             patch('urllib.request.build_opener') as transport:
            with self.assertRaises(ValueError):
                memory.system1_labels('orchard irrigation', ['a'], self.cfg)
            transport.assert_not_called()
        self.cfg['budget_usd_per_session'] = 0.1
        with patch.object(memory, 'system1_mask', side_effect=lambda x:x), \
             patch('urllib.request.build_opener') as transport:
            transport.return_value.open.side_effect = TimeoutError
            with self.assertRaises(ValueError):
                memory.system1_labels('orchard irrigation', ['a'], self.cfg)
        ledger = json.loads((self.home / 'system1' / 'usage.json').read_text())
        self.assertGreater(ledger['events'][0]['usd'],0)
        self.assertEqual(ledger['events'][0]['outcome'],'fallback')
        counter = next(iter(ledger['sessions'].values()))
        counter['attempts'] = 1000
        (self.home / 'system1' / 'usage.json').write_text(json.dumps(ledger))
        with patch.object(memory, 'system1_mask', side_effect=lambda x:x), \
             patch('urllib.request.build_opener') as transport:
            with self.assertRaises(ValueError):
                memory.system1_labels('orchard irrigation', ['other'], self.cfg)
            transport.assert_not_called()

    def test_weak_and_malformed_provider_judgments_abstain(self):
        weak = {'choice': 'direct', 'confidence': 0.5,
                'probabilities': {'direct':0.5,'supporting':0.3,'unrelated':0.1,'unknown':0.1}}
        with patch.object(memory, 'system1_mask', side_effect=lambda x:x), \
             patch('urllib.request.build_opener') as transport:
            transport.return_value.open.side_effect = lambda *a, **kw:self.response({'0':weak})
            self.assertEqual(memory.system1_labels('orchard irrigation', ['a'], self.cfg), ['unknown'])
            self.assertEqual(memory.system1_labels('orchard irrigation', ['a'], self.cfg), ['unknown'])
            self.assertEqual(transport.return_value.open.call_count, 2)
            weak['probabilities']['direct'] = float('nan')
            with self.assertRaises(ValueError):
                memory.system1_labels('orchard irrigation', ['a'], self.cfg)

    def test_command_streaming_cap_and_owner_filter(self):
        import argparse
        ns = self.home / 'namespaces' / 'synthetic'
        ns.mkdir(parents=True)
        (ns / 'a.md').write_text('orchard irrigation one\norchard irrigation dismissed\norchard irrigation two\norchard irrigation three\n')
        args = argparse.Namespace(root=str(self.home), ns='synthetic', all=False,
            pattern='orchard irrigation', ignore_case=False, max_lines=2, semantic=False)
        with patch.object(memory, 'owner_filter_line', side_effect=lambda root,line: 'dismissed' in line), \
             patch.object(memory, 'system1_rerank', side_effect=AssertionError('rerank called')):
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                memory.cmd_search(args)
            base = out.getvalue()
        self.assertNotIn('dismissed', base)
        self.assertIn('capped at 2', base)
        self.assertNotIn('three', base)
        args.semantic = True
        with patch.object(memory, 'owner_filter_line', side_effect=lambda root,line: 'dismissed' in line), \
             patch.object(memory, 'system1_rerank', side_effect=lambda q,h:(h,'disabled')):
            out = io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
                memory.cmd_search(args)
        self.assertEqual(out.getvalue(),base)


if __name__ == '__main__':
    unittest.main()
