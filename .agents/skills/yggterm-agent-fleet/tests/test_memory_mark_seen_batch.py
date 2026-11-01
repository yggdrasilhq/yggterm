#!/usr/bin/env python3
"""Hermetic acknowledgment projection checks; all journal records are invented."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import threading
import unittest
from unittest.mock import patch

SRC = Path(os.environ.get('MEMORY_ACK_SOURCE', Path(__file__).resolve().parents[1] / 'ygg-memory.py'))
spec = importlib.util.spec_from_file_location('memory_ack_under_test', SRC)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


def event(seq, origin='fixture-a', **kw):
    return dict(seq=seq, event_id=f'fixture-{seq}', origin=origin,
                ns='widgets', target_harness='all', **kw)


def oracle(records, watermark, namespace, harness):
    """Independent transcription of the original two-pass contract."""
    w = copy.deepcopy(watermark)
    vectors = w.setdefault('seen_origin_seq', {})
    records = [r for r in records if r.get('seq', 0) > 0]
    for r in records:
        if namespace is not None and r.get('ns') != namespace:
            continue
        if not m.matches_target_harness(r.get('target_harness', 'all'), harness):
            continue
        if r.get('event_id') and r.get('origin'):
            v = vectors.setdefault(r.get('ns', ''), {})
            v[r['origin']] = max(v.get(r['origin'], 0), r.get('seq', 0))
    covered = {m.record_event_id(r) for r in records if r.get('origin') and
               r.get('seq', 0) <= vectors.get(r.get('ns', ''), {}).get(r['origin'], 0)}
    w['seen_event_ids'] = sorted(set(w.get('seen_event_ids', [])) - covered)
    w['last_seq'] = max([0] + [r.get('seq', 0) for r in records])
    return w


class AckTests(unittest.TestCase):
    def setUp(self):
        scratch = Path(os.environ.get('TMPDIR', Path.home() / '.yggterm/scratchpad'))
        scratch.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix='ack-fixture-', dir=scratch)
        self.root = Path(self.temp.name)
        self.path = m.get_journal_path(self.root)
        self.path.parent.mkdir(parents=True, exist_ok=True)

    def tearDown(self):
        self.temp.cleanup()

    def write(self, records):
        self.path.write_text(''.join(json.dumps(r) + '\n' for r in records))

    def check_records(self, records, w=None, ns=None, harness='codex'):
        self.write(records)
        w = copy.deepcopy(w or {})
        expected = oracle(records, w, ns, harness)
        m.mark_events_seen(self.root, w, ns, harness)
        self.assertEqual(w, expected)
        return w

    def test_oracle_aliases_filters_duplicates_and_legacy(self):
        records = [event(8), event(8, 'fixture-b'), event(3), event(40)]
        records[2].update(event_id='fixture-8', ns='gadgets', target_harness='claude')
        records[3].update(ns='gadgets', target_harness='cc')
        records += [dict(seq=6, origin='fixture-a', ns='widgets', summary='legacy'),
                    dict(seq=1, event_id='unowned'),
                    dict(seq=2, event_id='missing-ns', origin='fixture-b'),
                    dict(seq=4, event_id='null-ns', origin='fixture-b', ns=None)]
        for target in ('all', '*', '', None, 'codex-litellm', ' Codex ', 'cc', 'claude_code'):
            for ns in (None, 'widgets', 'gadgets', '', 'absent'):
                for selective in ([], ['fixture-8', 'fixture-40', 'unmatched', m.record_event_id(records[4])]):
                    with self.subTest(target=target, ns=ns, selective=bool(selective)):
                        w = {'seen_origin_seq': {'unrelated': {'fixture-z': 90}, 'widgets': {'fixture-a': 7}},
                             'seen_event_ids': selective, 'other': 'preserved'}
                        self.check_records(records, w, ns, target)

    def test_unusual_metadata_fallback_and_zero_sequences(self):
        records = [event(0), event(-1), event(2), event(3), event(4)]
        records[2]['event_id'] = 123
        records[3]['event_id'] = ''
        records[4].pop('event_id')
        self.check_records(records, {'seen_event_ids': ['123', 'keep']})
        records[2]['event_id'] = None
        self.check_records(records, {'seen_event_ids': ['keep']})

    def test_duplicate_json_keys_use_python_last_value_semantics(self):
        self.path.write_text('{"seq": 3, "event_id": "old", "event_id": "new", '
                             '"origin": "fixture-a", "origin": "fixture-b", '
                             '"ns": "widgets", "target_harness": "claude", '
                             '"target_harness": "codex"}\n')
        records = [json.loads(self.path.read_text())]
        w = {'seen_event_ids': ['old', 'new']}
        expected = oracle(records, w, None, 'codex')
        m.mark_events_seen(self.root, w, None, 'codex')
        self.assertEqual(w, expected)

    def test_entry_target_normalization(self):
        records = []
        for i, target in enumerate(('all', '*', '', None, 'cc', 'claude_code', ' CLAUDE ', 'codex_litellm'), 1):
            r = event(i, f'fixture-{i}')
            r['target_harness'] = target
            records.append(r)
        for harness in ('claude', 'codex', 'all', '*', '', None):
            self.check_records(records, ns='widgets', harness=harness)

    def test_repeated_ids_selective_and_existing_vectors(self):
        records = [event(12), event(2), event(3, 'fixture-b')]
        for r in records:
            r.update(event_id='repeated', target_harness='claude')
        w = {'seen_origin_seq': {'widgets': {'fixture-a': 4}}, 'seen_event_ids': ['repeated', 'keep']}
        self.check_records(records, w, harness='codex')

    def test_missing_empty_rewrite_append_truncate(self):
        w = {}
        m.mark_events_seen(self.root, w, None, 'codex')
        self.assertEqual(w['last_seq'], 0)
        self.check_records([event(90)])
        self.path.unlink()
        m.mark_events_seen(self.root, w, None, 'codex')
        self.assertEqual(w['last_seq'], 0)
        for records in ([], [event(10)], [event(10), event(11)], [event(1)]):
            self.check_records(records)
        self.path.write_text(json.dumps(event(7)) + '\n{"seq":')
        w = {}
        m.mark_events_seen(self.root, w, None, 'codex')
        self.assertEqual(w['last_seq'], 7)
        self.path.write_text(json.dumps(event(9)))
        m.mark_events_seen(self.root, w, None, 'codex')
        self.assertEqual(w['last_seq'], 9)
        m._ji_invalidate(self.root)
        self.assertEqual(self.check_records([event(2)])['last_seq'], 2)

    def test_unavailable_index_json_and_corrupt_payload_fallback(self):
        records = [event(8), event(8, 'fixture-b')]
        self.write(records)
        expected = oracle(records, {}, None, 'codex')
        for name in ('_ji_connect', '_ji_refresh'):
            with patch.object(m, name, side_effect=sqlite3.OperationalError('fixture unavailable')):
                w = {}
                m.mark_events_seen(self.root, w, None, 'codex')
                self.assertEqual(w, expected)
        m.read_journal_entries(self.root)
        conn = m._ji_connect(self.root)
        conn.execute("UPDATE entries SET payload = 'broken-json'")
        conn.close()
        w = {}
        m.mark_events_seen(self.root, w, None, 'codex')
        self.assertEqual(w, expected)
        # A build without SQLite JSON functions still uses the coherent reader.
        original = m._ji_connect
        class NoJSON:
            def __init__(self, conn): self.conn = conn
            def __getattr__(self, name): return getattr(self.conn, name)
            def execute(self, sql, *args):
                if 'json_' in sql: raise sqlite3.OperationalError('no such function: json_type')
                return self.conn.execute(sql, *args)
        with patch.object(m, '_ji_connect', side_effect=lambda root: NoJSON(original(root))):
            w = {}
            m.mark_events_seen(self.root, w, None, 'codex')
            self.assertEqual(w, expected)
        m._journal_index_path(self.root).write_bytes(b'not a database')
        w = {}
        m.mark_events_seen(self.root, w, None, 'codex')
        self.assertEqual(w, expected)

    def test_failure_does_not_mutate_caller(self):
        w = {'seen_origin_seq': {'widgets': {'fixture-a': 'invalid'}}, 'seen_event_ids': ['keep']}
        before = copy.deepcopy(w)
        self.write([event(3, 'fixture-b'), event(4)])
        with self.assertRaises(TypeError):
            m.mark_events_seen(self.root, w, None, 'codex')
        self.assertEqual(w, before)
        with patch.object(m, '_ji_ack_projection', side_effect=ValueError('fixture')):
            with patch.object(m, '_read_journal_entries_naive', side_effect=OSError('fixture')):
                with self.assertRaises(OSError): m.mark_events_seen(self.root, w, None, 'codex')
        self.assertEqual(w, before)

    def test_decode_scaling_warm_10000_records(self):
        self.write([event(i, f'fixture-{i % 3}', summary='invented body' * 40) for i in range(1, 10001)])
        m.read_journal_entries(self.root)  # cold refresh is intentionally outside measurement
        with patch.object(m.json, 'loads', wraps=json.loads) as loads:
            with patch.object(m, '_read_journal_entries_naive', side_effect=AssertionError('unexpected fallback')):
                w = {'seen_event_ids': ['fixture-1', 'keep']}
                m.mark_events_seen(self.root, w, 'widgets', 'codex')
                self.assertEqual(w['last_seq'], 10000)
                self.assertEqual(w['seen_event_ids'], ['keep'])
            self.assertGreaterEqual(loads.call_count, 10000)
            self.assertLessEqual(loads.call_count, 10004)

    def test_refresh_snapshot_blocks_invalidation_and_excludes_later_legacy(self):
        self.write([event(1)])
        entered = threading.Event()
        done = threading.Event()
        threads = []
        refresh = m._ji_refresh
        def invalidate():
            entered.set()
            m._ji_invalidate(self.root)
            done.set()
        def hooked(root, conn):
            refresh(root, conn)
            self.assertTrue(conn.in_transaction)
            t = threading.Thread(target=invalidate)
            threads.append(t)
            t.start()
            self.assertTrue(entered.wait(1))
            self.assertFalse(done.wait(.05))
            with self.path.open('a') as out:
                out.write(json.dumps(dict(seq=2, ns='widgets', summary='later legacy')) + '\n')
        with patch.object(m, '_ji_refresh', side_effect=hooked):
            w = {}
            m.mark_events_seen(self.root, w, None, 'codex')
        for t in threads: t.join(3)
        self.assertTrue(done.is_set())
        self.assertEqual(w['last_seq'], 1)
        self.assertEqual(len(m.unseen_journal_entries(self.root, w, 'widgets', 'codex')), 1)

    def test_source_fallback_ceiling_is_consumed_snapshot(self):
        self.write([event(1)])
        naive = m._read_journal_entries_naive
        def read_then_append(root):
            records = naive(root)
            with self.path.open('a') as out:
                out.write(json.dumps(dict(seq=2, ns='widgets', summary='later legacy')) + '\n')
            return records
        with patch.object(m, '_ji_ack_projection', side_effect=ValueError('fixture')):
            with patch.object(m, '_read_journal_entries_naive', side_effect=read_then_append):
                w = {}
                m.mark_events_seen(self.root, w, None, 'codex')
        self.assertEqual(w['last_seq'], 1)
        self.assertEqual(len(m.unseen_journal_entries(self.root, w, 'widgets', 'codex')), 1)


if __name__ == '__main__':
    unittest.main()
