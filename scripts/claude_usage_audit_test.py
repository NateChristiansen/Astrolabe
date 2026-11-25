"""The usage audit counts each response once and never divides pooled usage among efforts."""
import datetime as dt
import json
import tempfile
import unittest
from pathlib import Path

import claude_usage_audit as audit

WEIGHTS = {'write': 1.25, 'long_write': 2.0, 'read': 0.1, 'out': 5.0}
SINCE = dt.datetime(2026, 1, 1, tzinfo=dt.timezone.utc)
UNTIL = dt.datetime(2100, 1, 1, tzinfo=dt.timezone.utc)


def response(ident, read, out=100, command=None, write=0, minute=0, tool_id='t1'):
    content = [{'type': 'text', 'text': 'ok'}]
    if command:
        content.append({'type': 'tool_use', 'id': tool_id, 'name': 'Bash', 'input': {'command': command}})
    return {'type': 'assistant', 'timestamp': f'2026-06-01T00:{minute:02d}:00Z', 'entrypoint': 'cli',
            'message': {'id': ident, 'model': 'model-a', 'content': content,
                        'usage': {'input_tokens': 10, 'cache_creation_input_tokens': write,
                                  'cache_read_input_tokens': read, 'output_tokens': out}}}


def result(tool_id, text):
    return {'type': 'user', 'message': {'content': [
        {'type': 'tool_result', 'tool_use_id': tool_id, 'content': text}]}}


def bind(ident, effort, minute=0, worker=False, read=1000):
    """An activation and the result that confirms it."""
    flags = ' --worker --model m --reasoning medium' if worker else ''
    return [response(ident, read, command=f'astrolabe activate --effort {effort}{flags}',
                     minute=minute, tool_id=ident),
            result(ident, f'Astrolabe is on in this session for effort {effort}, "x".')]


def write(path, entries):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(''.join(json.dumps(e, separators=(',', ':')) + '\n' for e in entries), encoding='utf-8')


class AuditTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))

    def report(self):
        return audit.audit(str(self.root), SINCE, UNTIL, WEIGHTS)

    def test_a_response_logged_per_content_block_counts_once(self):
        write(self.root / 'p/s1.jsonl', [response('m1', 1000), response('m1', 1000), response('m2', 2000, minute=1)])
        total = self.report()['total']
        self.assertEqual((total['requests'], total['read']), (2, 3000))

    def test_a_worker_is_recognised_and_its_usage_goes_to_its_effort(self):
        write(self.root / 'p/s1.jsonl', [
            *bind('m1', 12, worker=True, read=500_000),
            response('m2', 500_000, command='sleep 30', minute=1)])
        report = self.report()
        self.assertEqual(report['by_role']['worker']['requests'], 2)
        self.assertEqual(report['by_effort']['12']['read'], 1_000_000)
        self.assertEqual(report['by_context']['400-700k']['requests'], 2)
        self.assertEqual((report['counts']['polls'], report['counts']['past_400k']), (1, 1))

    def test_an_effort_created_in_the_session_counts_as_bound(self):
        write(self.root / 'p/s1.jsonl', [
            response('m1', 1000, command='astrolabe effort new --title "x"', tool_id='t9'),
            result('t9', 'Effort 77 created as efforts/x.md')])
        self.assertIn('77', self.report()['by_effort'])

    def test_a_refused_activation_binds_nothing(self):
        write(self.root / 'p/s1.jsonl', [
            response('m1', 1000, command='astrolabe activate --effort 9 --worker', tool_id='t3'),
            result('t3', 'NOT recorded: the hook did not take this command.')])
        report = self.report()
        self.assertEqual(set(report['by_effort']), {'unbound'})
        self.assertEqual(set(report['by_role']), {'no effort'})

    def test_pooled_usage_is_not_divided(self):
        write(self.root / 'p/two.jsonl', [*bind('m1', 1), *bind('m2', 2, minute=1)])
        write(self.root / 'p/none.jsonl', [response('m3', 1000)])
        efforts = self.report()['by_effort']
        self.assertEqual(set(efforts), {'shared', 'unbound'})
        self.assertEqual(efforts['shared']['requests'], 2)

    def test_a_subagent_counts_toward_the_session_that_started_it(self):
        write(self.root / 'p/s1.jsonl', bind('m1', 5))
        write(self.root / 'p/s1/subagents/agent-a.jsonl', [response('m2', 4000)])
        report = self.report()
        self.assertEqual(report['by_effort']['5']['read'], 5000)
        self.assertEqual((report['counts']['sessions'], report['counts']['subagent_requests']), (1, 1))

    def test_a_cap_on_cached_context_only_lowers_the_read_share(self):
        write(self.root / 'p/s1.jsonl', [response('m1', 1_000_000, out=0)])
        capped = self.report()['if_cached_context_capped']
        # 10 fresh + 0.1 x 150k against 10 + 0.1 x 1M.
        self.assertAlmostEqual(capped['150000'], 15_010 / 100_010, places=3)

    def test_requests_outside_the_window_are_left_out(self):
        write(self.root / 'p/s1.jsonl', [response('m1', 1000)])
        late = dt.datetime(2026, 7, 1, tzinfo=dt.timezone.utc)
        self.assertEqual(audit.audit(str(self.root), late, UNTIL, WEIGHTS)['total']['requests'], 0)


if __name__ == '__main__':
    unittest.main()
