#!/usr/bin/env python3
"""Where Claude Code usage went, from the transcripts on this machine.

Reads the session transcripts under ~/.claude/projects and prints raw token counts
(fresh input, cache writes, cache reads, output) with request counts, split by
context size, by Astrolabe role and by effort. It reads local files only and writes
nothing; pass --json for a machine-readable report to diff between runs.

Raw counts are the result. The "weighted" column applies price-like ratios so the
categories can be added up; nobody outside the provider knows how a subscription
counts a re-read token, so change --cache-read-weight and see whether a conclusion
survives before leaning on it.

A session counts toward an effort when its `astrolabe activate` or `astrolabe effort
new` command came back saying the effort is on or was created. A session bound to more than one effort, or
to an initiative, goes in the "shared" bucket, and one bound to nothing in "unbound":
pooled usage is reported as pooled, never divided among efforts.
"""
import argparse
import collections
import datetime as dt
import json
import os
import re
import statistics
import sys

BUCKETS = ((50_000, '<50k'), (100_000, '50-100k'), (200_000, '100-200k'),
           (400_000, '200-400k'), (700_000, '400-700k'), (float('inf'), '700k+'))
CAPS = (150_000, 250_000, 400_000)
ASTROLABE = re.compile(r'astrolabe(?:\.exe)?\s+([a-z][a-z-]*)(?:\s+([a-z][a-z-]*))?')
GROUPED = {'effort', 'initiative', 'lesson', 'notes'}
BOUND = re.compile(r'is on in this session for effort (\d+)\b')
INITIATIVE = re.compile(r'is on in this session for initiative\b')
CREATED = re.compile(r'\bEffort (\d+) created\b')
POLL = re.compile(r'(?:^|[;&|]\s*|\bdo\s+)(?:sleep|Start-Sleep|timeout)\b')


def bucket(context):
    return next(name for limit, name in BUCKETS if context < limit)


def text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return ''.join(p.get('text') or '' for p in content if isinstance(p, dict))
    return ''


def read_session(path):
    """One transcript file -> its requests and what the session did."""
    requests = {}
    facts = {'worker': False, 'efforts': set(), 'initiative': False, 'entry': None,
             'polls': 0, 'compactions': 0, 'astrolabe': collections.Counter()}
    binding = {}
    with open(path, encoding='utf-8', errors='replace') as lines:
        for line in lines:
            if '"type":"assistant"' not in line and '"type":"user"' not in line \
                    and 'compact_boundary' not in line:
                continue
            try:
                entry = json.loads(line)
            except ValueError:
                continue
            kind = entry.get('type')
            if kind == 'system' and entry.get('subtype') == 'compact_boundary':
                facts['compactions'] += 1
                continue
            if facts['entry'] is None and entry.get('entrypoint'):
                facts['entry'] = entry['entrypoint']
            message = entry.get('message') or {}
            content = message.get('content')
            if kind == 'user':
                # A binding counts only when the command's own result confirms it:
                # a refused activation binds nothing.
                for part in content if isinstance(content, list) else ():
                    if not isinstance(part, dict) or part.get('tool_use_id') not in binding:
                        continue
                    worker = binding.pop(part['tool_use_id'])
                    said = text_of(part.get('content'))
                    bound = BOUND.findall(said) + CREATED.findall(said)
                    facts['efforts'].update(bound)
                    initiative = bool(INITIATIVE.search(said))
                    facts['initiative'] |= initiative
                    facts['worker'] |= worker and bool(bound or initiative)
                continue
            if kind != 'assistant':
                continue
            tools = []
            for part in content if isinstance(content, list) else ():
                if not isinstance(part, dict) or part.get('type') != 'tool_use':
                    continue
                tools.append(part.get('name'))
                command = (part.get('input') or {}).get('command')
                if part.get('name') not in ('Bash', 'PowerShell') or not command:
                    continue
                if POLL.search(command):
                    facts['polls'] += 1
                found = ASTROLABE.search(command)
                if not found:
                    continue
                verb, noun = found.groups()
                facts['astrolabe'][f'{verb} {noun}' if verb in GROUPED and noun else verb] += 1
                if verb == 'activate' or (verb == 'effort' and noun == 'new'):
                    binding[part.get('id')] = '--worker' in command
            usage = message.get('usage')
            if not usage or message.get('model') in (None, '<synthetic>'):
                continue
            # A response is logged once per content block; the last line has the final usage.
            row = requests.setdefault(message.get('id') or entry.get('uuid'), {
                'time': entry.get('timestamp'), 'model': message['model'], 'tools': 0,
                'side': bool(entry.get('isSidechain'))})
            row['tools'] += len(tools)
            split = usage.get('cache_creation') or {}
            long_write = split.get('ephemeral_1h_input_tokens') or 0
            row.update(fresh=usage.get('input_tokens') or 0,
                       write=usage.get('cache_creation_input_tokens') or 0,
                       long_write=long_write,
                       read=usage.get('cache_read_input_tokens') or 0,
                       out=usage.get('output_tokens') or 0)
    return list(requests.values()), facts


def sessions(root, since, until):
    """Yield (name, is_subagent_file, requests, facts) for transcripts touched in the window."""
    for folder, _, files in os.walk(root):
        for name in files:
            if not name.endswith('.jsonl'):
                continue
            path = os.path.join(folder, name)
            try:
                if os.path.getmtime(path) < since.timestamp():
                    continue
            except OSError:
                continue
            requests, facts = read_session(path)
            requests = [r for r in requests if r['time'] and since <= when(r['time']) < until]
            if requests:
                sub = 'subagents' in os.path.relpath(path, root).split(os.sep) or name.startswith('agent-')
                yield path, sub, requests, facts


def when(stamp):
    return dt.datetime.fromisoformat(stamp.replace('Z', '+00:00'))


class Tally:
    """Raw token categories and a request count for one slice of usage."""
    fields = ('requests', 'fresh', 'write', 'long_write', 'read', 'out')

    def __init__(self):
        self.requests = self.fresh = self.write = self.long_write = self.read = self.out = 0

    def add(self, row):
        self.requests += 1
        for field in self.fields[1:]:
            setattr(self, field, getattr(self, field) + row[field])

    def weighted(self, weights, read=None):
        short = self.write - self.long_write
        return (self.fresh + weights['write'] * short + weights['long_write'] * self.long_write
                + weights['read'] * (self.read if read is None else read) + weights['out'] * self.out)

    def as_dict(self, weights):
        return {**{f: getattr(self, f) for f in self.fields}, 'weighted': round(self.weighted(weights))}


def audit(root, since, until, weights):
    total = Tally()
    by = {name: collections.defaultdict(Tally) for name in
          ('context', 'role', 'model', 'effort', 'entry')}
    capped = dict.fromkeys(CAPS, 0.0)
    starts = collections.defaultdict(list)
    calls = collections.Counter()
    seen = {'sessions': 0, 'past_400k': 0, 'polls': 0, 'compactions': 0, 'single_tool': 0,
            'short_reply': 0, 'subagent_requests': 0}
    whole_sessions = collections.defaultdict(lambda: [Tally(), 0])
    found = list(sessions(root, since, until))
    parents = {path: facts for path, sub, _, facts in found if not sub}
    for path, sub, requests, own in found:
        requests.sort(key=lambda r: r['time'])
        # A subagent's usage belongs to the session that started it:
        # <session>/subagents/<agent>.jsonl sits beside <session>.jsonl.
        parent = os.path.dirname(os.path.dirname(path)) + '.jsonl' if sub else path
        facts = parents.get(parent, own)
        if facts['worker']:
            role = 'worker'
        elif facts['efforts'] or facts['initiative']:
            role = 'effort-bound, not a worker'
        else:
            role = 'no effort'
        if facts['initiative'] or len(facts['efforts']) > 1:
            effort = 'shared'
        else:
            effort = next(iter(facts['efforts']), 'unbound')
        session = whole_sessions[parent]
        for row in requests:
            context = row['fresh'] + row['write'] + row['read']
            session[1] = max(session[1], context)
            single = Tally()
            single.add(row)
            for cap in CAPS:
                capped[cap] += single.weighted(weights, read=min(row['read'], cap))
            for tally in (total, session[0], by['context'][bucket(context)], by['role'][role],
                          by['model'][row['model']], by['effort'][effort],
                          by['entry'][facts['entry'] or 'unknown']):
                tally.add(row)
            seen['single_tool'] += row['tools'] == 1
            seen['short_reply'] += row['out'] < 300
            seen['subagent_requests'] += sub or row['side']
        if not sub:
            seen['sessions'] += 1
            starts[facts['entry'] or 'unknown'].append(
                requests[0]['fresh'] + requests[0]['write'] + requests[0]['read'])
            calls.update(facts['astrolabe'])
            seen['polls'] += facts['polls']
            seen['compactions'] += facts['compactions']
    past_400k = Tally()
    for session, peak in whole_sessions.values():
        if peak > 400_000:
            seen['past_400k'] += 1
            for field in Tally.fields:
                setattr(past_400k, field, getattr(past_400k, field) + getattr(session, field))
    whole = total.weighted(weights) or 1
    return {
        'window': {'since': since.isoformat(), 'until': until.isoformat()},
        'weights': weights,
        'total': total.as_dict(weights),
        'mean_context': round((total.fresh + total.write + total.read) / max(total.requests, 1)),
        'counts': seen,
        'sessions_past_400k': past_400k.as_dict(weights),
        'if_cached_context_capped': {str(cap): round(value / whole, 3) for cap, value in capped.items()},
        'first_request_context': {entry: {'sessions': len(sizes), 'median': round(statistics.median(sizes))}
                                  for entry, sizes in starts.items()},
        'astrolabe_calls': dict(calls.most_common(12)),
        **{f'by_{name}': {key: tally.as_dict(weights) for key, tally in groups.items()}
           for name, groups in by.items()},
    }


def millions(count):
    return f'{count / 1e6:,.1f}M'


def table(title, groups, whole, order=None, limit=None):
    print(f'\n{title}')
    print(f'  {"":28} {"requests":>9} {"cache read":>11} {"cache write":>11} {"output":>9} {"weighted":>9}')
    keys = order or sorted(groups, key=lambda key: -groups[key]['weighted'])
    for key in keys[:limit]:
        row = groups.get(key)
        if row:
            print(f'  {str(key)[:28]:28} {row["requests"]:>9,} {millions(row["read"]):>11} '
                  f'{millions(row["write"]):>11} {millions(row["out"]):>9} {row["weighted"] / whole:>9.1%}')


def show(report):
    total, counts = report['total'], report['counts']
    whole = total['weighted'] or 1
    requests = total['requests'] or 1
    window = report['window']
    print(f'Claude Code usage, {window["since"][:10]} to {window["until"][:10]}')
    print(f'  {total["requests"]:,} requests in {counts["sessions"]:,} sessions; '
          f'mean context per request {report["mean_context"]:,} tokens')
    print(f'  raw tokens: cache read {millions(total["read"])}, cache write {millions(total["write"])}, '
          f'fresh input {millions(total["fresh"])}, output {millions(total["out"])}')
    print(f'  weights used for the weighted column: {report["weights"]}')
    table('By context size at the request', report['by_context'], whole,
          order=[name for _, name in BUCKETS])
    past = report['sessions_past_400k']
    print(f'\n  {counts["past_400k"]:,} sessions ever passed 400k context; '
          f'they hold {past["weighted"] / whole:.1%} of weighted usage')
    print('  if cached context had never passed a cap, same requests (a ceiling, not a forecast):')
    for cap, share in report['if_cached_context_capped'].items():
        print(f'    {int(cap) // 1000}k: {share:.0%} of actual')
    table('By Astrolabe role', report['by_role'], whole)
    table('By model', report['by_model'], whole)
    table('By effort (largest; shared and unbound are pooled, not divided)',
          report['by_effort'], whole, limit=15)
    print('\nSession start size (first request, median tokens)')
    for entry, start in sorted(report['first_request_context'].items()):
        print(f'  {entry:28} {start["median"]:>9,}  ({start["sessions"]} sessions)')
    print('\nCandidates to look at, not a count of waste')
    print(f'  requests with one tool call: {counts["single_tool"]:,} ({counts["single_tool"] / requests:.0%})')
    print(f'  requests with under 300 output tokens: {counts["short_reply"]:,} ({counts["short_reply"] / requests:.0%})')
    print(f'  shell calls that sleep or poll: {counts["polls"]:,}')
    print(f'  compactions: {counts["compactions"]:,}')
    print(f'  requests made by subagents: {counts["subagent_requests"]:,}')
    print('  astrolabe calls: ' + ', '.join(f'{name} {n:,}' for name, n in report['astrolabe_calls'].items()))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    parser.add_argument('--root', default=os.path.join(os.path.expanduser('~'), '.claude', 'projects'))
    parser.add_argument('--days', type=int, default=35, help='look back this many days (default 35)')
    parser.add_argument('--since', help='start date, YYYY-MM-DD (UTC); overrides --days')
    parser.add_argument('--until', help='end date, YYYY-MM-DD (UTC), exclusive; default now')
    parser.add_argument('--cache-read-weight', type=float, default=0.1)
    parser.add_argument('--json', action='store_true', help='print the report as JSON')
    args = parser.parse_args(argv)
    utc = dt.timezone.utc
    until = dt.datetime.fromisoformat(args.until).replace(tzinfo=utc) if args.until else dt.datetime.now(utc)
    since = (dt.datetime.fromisoformat(args.since).replace(tzinfo=utc) if args.since
             else until - dt.timedelta(days=args.days))
    weights = {'write': 1.25, 'long_write': 2.0, 'read': args.cache_read_weight, 'out': 5.0}
    report = audit(args.root, since, until, weights)
    if args.json:
        json.dump(report, sys.stdout, indent=1)
        print()
    else:
        show(report)


if __name__ == '__main__':
    main()
