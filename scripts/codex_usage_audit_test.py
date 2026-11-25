import json
from datetime import datetime, timezone
from pathlib import Path
import tempfile
import unittest

from codex_usage_audit import summarize


def event(kind, payload, second=0, day=18):
    return {'type': kind, 'payload': payload,
            'timestamp': f'2026-11-{day:02d}T00:00:{second:02d}Z'}


def usage(response, size=100, cached=90, **time):
    return event('token_usage_record', {'response_id': response, 'usage': {
        'input_tokens': size, 'cached_input_tokens': cached, 'output_tokens': 10,
        'reasoning_output_tokens': 4}}, **time)


def call(cid, name, source, **time):
    fields = {'call_id': cid, 'type': 'custom_tool_call', 'name': name, 'input': source}
    if name == 'wait':
        del fields['input']
        fields.update(type='function_call', arguments=json.dumps({'cell_id': source}))
    return event('response_item', fields, **time)


def result(cid, output, **time):
    return event('response_item', {'type': 'function_call_output', 'call_id': cid,
                                   'output': output}, **time)


class AuditTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)

    def write(self, name, entries):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(''.join(json.dumps(e) + '\n' for e in entries), encoding='utf8')

    def audit(self):
        return summarize(self.root, datetime(2026, 11, 18, tzinfo=timezone.utc),
                         datetime(2026, 11, 25, tzinfo=timezone.utc))

    def test_metering_is_unique_and_dates_filter_responses_not_files(self):
        entries = [event('session_meta', {'id': 'a'}, day=17),
                   event('turn_context', {'model': 'synthetic'}, day=17),
                   usage('old', day=17), usage('one'), event('compacted', {}),
                   event('event_msg', {'type': 'token_count', 'info': {'total_token_usage': {}}}),
                   usage('two', 150, 110), usage('bad', 5, 10), usage('later', day=25)]
        self.write('sessions/a.jsonl', entries)
        self.write('archived_sessions/copy.jsonl', entries)
        self.write('sessions/legacy.jsonl', [event('event_msg', {'type': 'token_count'})])
        r = self.audit()
        self.assertEqual(r['totals'], dict(requests=2, fresh_input=50, cached_input=200, output=20))
        self.assertEqual(r['by_model']['synthetic'], r['totals'])
        self.assertEqual(r['by_effort']['unbound'], r['totals'])
        self.assertEqual(r['input_per_request']['mean'], 125)
        self.assertEqual(r['input_before_compaction']['median'], 100)
        self.assertEqual(r['compaction_markers'], 1)
        self.assertEqual(r['coverage']['observed_sessions'], 2)
        self.assertEqual(r['coverage']['metered_sessions'], 1)

    def test_only_confirmed_bindings_count_and_shared_is_never_split(self):
        def bind(number, day=18):
            cid = f'bind-{number}'
            return [call(cid, 'exec', f'tools.exec_command({{cmd:"astrolabe activate --effort {number} --worker"}})', day=day),
                    result(cid, f'Astrolabe is on in this session for effort {number},', day=day)]
        self.write('sessions/a.jsonl', bind(7, day=17) + [usage('a')])
        self.write('sessions/b.jsonl', bind(7) + bind(8) + [usage('b')])
        self.write('sessions/c.jsonl', [call('x', 'exec', 'tools.exec_command({cmd:"astrolabe activate --initiative demo"})'),
                                        result('x', 'Astrolabe is on in this session for initiative demo,'), usage('c')])
        self.write('sessions/d.jsonl', [*bind(7)[:1], result('bind-7', 'Refused'), usage('d')])
        r = self.audit()
        self.assertEqual({k: v['requests'] for k, v in r['by_effort'].items()},
                         {'7': 1, 'shared': 2, 'unbound': 1})
        self.assertEqual(r['by_role']['worker']['requests'], 2)

    def test_candidates_pair_results_and_preserve_final_usage(self):
        entries = []
        for i, (cell, finish, delay) in enumerate([
                ('a', 'Script completed', 15), ('wrong', 'Script completed', 15),
                ('a', 'Script completed', 41), ('a', 'Script running with cell ID a\nWall time 1s\nOutput:\n', 4)]):
            entries.extend([event('event_msg', {'type': 'task_started'}),
                            call('e'+str(i), 'exec', '// @exec: {"yield_time_ms": 1000}\nwork()'),
                            usage('exec'+str(i)), result('e'+str(i), 'Script running with cell ID a\n'),
                            call('w'+str(i), 'wait', cell, second=2), usage('wait'+str(i), second=2),
                            result('w'+str(i), [{'type': 'input_text', 'text': finish}], second=delay)])
        self.write('sessions/jobs.jsonl', entries)
        r = self.audit()
        self.assertEqual(r['totals']['requests'], 8)
        for k in ['early_yield_then_wait', 'empty_wait']:
            self.assertEqual(r['wait_candidates'][k]['requests'], 1)
            self.assertEqual(r['wait_candidates'][k]['fresh_input'], 10)
