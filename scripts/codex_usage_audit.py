#!/usr/bin/env python3
"""Read local Codex rollouts and summarize recorded model responses, without writes.

Dates are UTC, with an exclusive end. A response_id is counted once across active,
archived and copied histories. token_count snapshots are not additional requests.
Only paired action/results from this session establish an Astrolabe binding.
Multiple efforts or an initiative are shared. These buckets describe sessions,
not accepted outcomes or subscription charges.

An initiative binding takes the dispatcher role; otherwise a confirmed worker
binding takes the worker role. Unknown includes ordinary effort-bound sessions
and sessions without a binding. Use by_effort to distinguish those two when
comparing with the Claude report's effort-bound and no-effort categories.
"""
import argparse
from collections import Counter, defaultdict
from datetime import datetime, timedelta, timezone
import json
import math
import os
from pathlib import Path
import re
import statistics


FIELDS = ('requests', 'cached_input', 'fresh_input', 'output')
BUCKETS = (50_000, 100_000, 200_000, 400_000, 700_000)
RUNNING = re.compile(r'^Script running with cell ID (\S+)\s*\n')
EMPTY = re.compile(r'^Script running with cell ID \S+\s*\nWall time [^\n]+\nOutput:\s*$')
BIND = re.compile(r"""(?:^\s*|\bcmd\s*:\s*["'`])astrolabe\s+(activate\s+--(effort|initiative)\s+([\w-]+)|effort\s+new\b)""", re.I)


def timestamp(value):
    return datetime.fromisoformat(value.replace('Z', '+00:00')).astimezone(timezone.utc)


def output_text(value):
    if isinstance(value, list):
        return '\n'.join(p.get('text', '') for p in value if isinstance(p, dict))
    return value if isinstance(value, str) else ''


def counters():
    return Counter(dict.fromkeys(FIELDS, 0))


def size_summary(values):
    if not values:
        return {'count': 0}
    ordered = sorted(values)
    return {'count': len(values), 'mean': round(statistics.mean(values), 1),
            'median': statistics.median(ordered), 'p90': ordered[math.ceil(.9 * len(values)) - 1],
            'max': ordered[-1]}


def context_bucket(size):
    lower = 0
    for upper in BUCKETS:
        if size < upper:
            return f'{lower // 1000}-{upper // 1000}k'
        lower = upper
    return '700k+'


def summarize(root, since, until):
    seen = set()
    sessions = defaultdict(counters)
    efforts = defaultdict(set)
    initiatives = set()
    workers = set()
    models = defaultdict(counters)
    buckets = defaultdict(counters)
    candidates = defaultdict(counters)
    contexts, before = [], []
    compacted = set()
    coverage = Counter()
    present = set()
    for folder in ('sessions', 'archived_sessions'):
        for path in sorted((root / folder).rglob('*.jsonl')):
            tid = path.stem
            model = 'unknown'
            previous = running = last_size = None
            bindings, waits, annotations = {}, {}, {}
            wait_calls = set()
            pending, attributed = [], []
            with path.open(encoding='utf-8', errors='replace') as lines:
                for line in lines:
                    try:
                        entry = json.loads(line)
                        when = timestamp(entry['timestamp'])
                    except (ValueError, KeyError, TypeError, AttributeError):
                        coverage['unreadable_records'] += 1
                        continue
                    if when >= until:
                        continue
                    in_window = when >= since
                    kind = entry.get('type')
                    v = entry.get('payload') or {}
                    if not isinstance(v, dict):
                        continue
                    typ = v.get('type')
                    if kind == 'session_meta':
                        tid = v.get('id') or tid
                    if in_window:
                        present.add(tid)
                    if kind == 'turn_context':
                        model = v.get('model') or 'unknown'
                    if kind == 'compacted':
                        key = (tid, when)
                        if in_window and key not in compacted:
                            compacted.add(key)
                            if last_size is not None:
                                before.append(last_size)
                        last_size = None
                    if kind == 'event_msg' and typ == 'task_started':
                        previous = running = None
                        pending.clear()
                    if kind == 'response_item' and typ in ('custom_tool_call', 'function_call'):
                        name, cid = v.get('name'), v.get('call_id')
                        source = v.get('input') or v.get('arguments') or ''
                        try:
                            args = json.loads(source)
                        except (ValueError, TypeError):
                            args = {}
                        args = args if isinstance(args, dict) else {}
                        if name in ('exec', 'exec_command'):
                            bind = BIND.search(args.get('cmd', source))
                            if bind:
                                bindings[cid] = (bind[2], bind[3], '--worker' in source)
                        if name == 'wait':
                            wait_calls.add(cid)
                            cell = args.get('cell_id')
                            if cell and running and previous == running[0] and cell == running[1]:
                                waits[cid] = running[2]
                        previous = cid
                        pending.append(cid)
                        header = re.match(r'^\s*//\s*@exec:\s*({[^\n]+})', source)
                        running = None
                        if name == 'exec' and header:
                            try:
                                if 0 < json.loads(header[1]).get('yield_time_ms', 0) <= 1000:
                                    running = (cid, None, when)
                            except (ValueError, TypeError, AttributeError):
                                pass
                    if kind == 'response_item' and typ in ('custom_tool_call_output', 'function_call_output'):
                        cid = v.get('call_id')
                        out = output_text(v.get('output'))
                        if cid in bindings:
                            verb, number, worker = bindings.pop(cid)
                            if verb == 'initiative' and re.search(r'is on in this session for initiative', out, re.I):
                                initiatives.add(tid)
                            match = re.search(r'is on in this session for effort (\d+)\b', out, re.I)
                            if verb is None:
                                match = re.search(r'\bEffort (\d+) created\b', out, re.I)
                            if match and (verb is None or number == match[1]):
                                efforts[tid].add(match[1])
                                if worker:
                                    workers.add(tid)
                        if running and running[0] == cid:
                            marker = RUNNING.match(out)
                            if marker:
                                running = (cid, marker[1], running[2])
                        if EMPTY.fullmatch(out):
                            annotations[cid] = 'empty_wait'
                        elif cid in waits and out.startswith('Script completed') and 0 <= (when - waits[cid]).total_seconds() <= 30:
                            annotations[cid] = 'early_yield_then_wait'
                    if kind != 'token_usage_record':
                        continue
                    actions, pending = pending, []
                    rid, u = v.get('response_id'), v.get('usage') or {}
                    numbers = [u.get(k) for k in ('input_tokens', 'cached_input_tokens', 'output_tokens')]
                    if not rid or any(type(i) is not int or i < 0 for i in numbers) or numbers[1] > numbers[0]:
                        if in_window:
                            coverage['invalid_usage_records'] += 1
                        continue
                    last_size, cached, output = numbers
                    if not in_window or rid in seen:
                        continue
                    seen.add(rid)
                    row = dict(zip(FIELDS, (1, cached, last_size - cached, output)))
                    contexts.append(last_size)
                    sessions[tid].update(row)
                    models[model].update(row)
                    buckets[context_bucket(last_size)].update(row)
                    if len(actions) == 1 and actions[0] in wait_calls:
                        attributed.append((actions[0], row))
            for cid, row in attributed:
                if cid in annotations:
                    candidates[annotations[cid]].update(row)
    by_effort, roles = defaultdict(counters), defaultdict(counters)
    for tid, counts in sessions.items():
        bound = efforts[tid]
        group = 'shared' if tid in initiatives or len(bound) > 1 else next(iter(bound), 'unbound')
        by_effort[group].update(counts)
        role = 'dispatcher' if tid in initiatives else 'worker' if tid in workers else 'unknown'
        roles[role].update(counts)
    total = counters()
    for counts in sessions.values():
        total.update(counts)
    return {'window_utc': [since.isoformat(), until.isoformat()], 'totals': total,
            'coverage': dict(coverage, observed_sessions=len(present), metered_sessions=len(sessions)),
            'input_per_request': size_summary(contexts), 'input_before_compaction': size_summary(before),
            'compaction_markers': len(compacted), 'by_effort': dict(by_effort), 'by_role': dict(roles),
            'by_model': dict(models), 'by_context_size': dict(buckets),
            'wait_candidates': {k: candidates[k] for k in ('early_yield_then_wait', 'empty_wait')}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(os.environ.get('CODEX_HOME') or Path.home() / '.codex'))
    parser.add_argument('--days', type=int, default=35)
    parser.add_argument('--since', type=datetime.fromisoformat, help='inclusive UTC date')
    parser.add_argument('--until', type=datetime.fromisoformat, help='exclusive UTC date')
    parser.add_argument('--json', action='store_true')
    args = parser.parse_args()
    def utc(value):
        return value.replace(tzinfo=timezone.utc) if value.tzinfo is None else value.astimezone(timezone.utc)
    end = utc(args.until) if args.until else datetime.now(timezone.utc)
    start = utc(args.since) if args.since else end - timedelta(days=args.days)
    if start >= end or not any((args.root / d).is_dir() for d in ('sessions', 'archived_sessions')):
        parser.error('provide a valid date range and a Codex home containing session logs')
    report = summarize(args.root, start, end)
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        for name, values in report.items():
            print(name.replace('_', ' '), json.dumps(values, sort_keys=True))
        print('Wait candidates are observations, not measured savings. Unmetered sessions contribute no tokens.')


if __name__ == '__main__':
    main()
