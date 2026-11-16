"""Q differential: 2000 seeded random scenarios (seed 20261116) plus fixed probes, run on the pinned TypeScript (Node) and
on the four pairs built by test_impl.py. Results and the requests the fake recorded must match; a difference is accepted
only when oracle.classify names one of the spec's policies (P1, P2, P3, P4, P6, P8). Only wall time is measured.

    python3 bench/l3/codex/q.py [N]        (N random cases, default 2000; needs test_impl.py to have built the runners)
"""
import sys
sys.dont_write_bytecode = True
import collections
import datetime
import json
import random
import time
from common import HERE, WORK, fake_binary, request_for, read_log, run_lines, node_command, calls_of, violations_of
from oracle import classify

SEED = 20261116
PAIRS = [(c, b) for c in ('py', 'rs') for b in ('rust', 'llvm')]
THREADS = ['t-1', 'abc_DEF-9', 'a', 'x' * 128, '../x', '', 'a b', 'x' * 129, '-lead', 'é', '1']
CWDS = ['/work/proj', '/work/proj/', '/work/proj//', '/Work/Proj', '/other', '', '/', '//']
TEXTS = ['', ' ', 'hello', 'a\tb', 'line1\nline2', '  padded  ', '한글', '😀', 'é' * 30, ' nb　', 'z' * 300, 'a\r\nb', ' ', '﻿ x', '​z',
         'x', 'yy', 'The quick brown fox', '{"json":"in text"}', '- not a plan', 'tab\t\tsep']
LONG = ['y' * 33000, '日' * 15000, 'é' * 20000, 'w' * 70000]


def rtext(r):
    k = r.random()
    if k < 0.06:
        return r.choice(LONG)
    return r.choice(TEXTS)


def rtime(r):
    k = r.randrange(14)
    if k == 0:
        return None  # absent (caller drops)
    if k == 1:
        return r.randrange(1, 2_000_000_000)
    if k == 2:
        return r.randrange(100_000_000_000, 1_900_000_000_000)
    if k == 3:
        return -r.randrange(1, 100000)
    if k == 4:
        return str(r.randrange(1, 2_000_000_000))
    if k == 5:
        return datetime.datetime.fromtimestamp(r.randrange(0, 2_000_000_000), datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%S.') + '%03dZ' % r.randrange(1000)
    if k == 6:
        return r.choice(['garbage', '', '  ', 'x1', '12ab'])
    if k == 7:
        return r.choice([True, False, None, [], {}])
    if k == 8:
        return r.randrange(1, 2_000_000_000) + 0.5  # P4
    if k == 9:
        return datetime.datetime.fromtimestamp(r.randrange(0, 2_000_000_000), datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')  # P4
    if k == 10:
        return r.randrange(10**12, 9 * 10**12)
    return r.randrange(1, 2_000_000_000)


def with_time(r, d, key):
    t = rtime(r)
    if t is not None:
        d[key] = t


def rplan(r):
    lines = []
    for _ in range(r.randrange(0, 40) if r.random() < 0.15 else r.randrange(0, 6)):
        mark = r.choice(['', '- ', '* ', '1. ', '12. ', '-', '3.x ', '  - ', '\t* ', '- \t'])
        body = r.choice(TEXTS + ['q' * 250, 'step ' + str(r.randrange(100))])
        lines.append(mark + body)
    return r.choice(['\n', '\r\n']).join(lines)


def ritem(r):
    t = r.choice(['userMessage'] * 4 + ['agentMessage'] * 4 + ['plan', 'plan', 'reasoning', 'reasoning', 'commandExecution', 'fileChange', 'mcpToolCall', None, 5, 'weird'])
    item = {}
    if t is not None:
        item['type'] = t
    form = r.randrange(5)
    if form == 0:
        item['text'] = rtext(r) if t != 'plan' else rplan(r)
    elif form == 1:
        item['content'] = [r.choice([{'text': rtext(r)}, {'text': rtext(r), 'type': 'input_text'}, {'x': 1}, {'text': 5}, 5, None, {'text': ''}]) for _ in range(r.randrange(0, 4))]
    elif form == 2:
        item['text'] = rtext(r)
        item['content'] = [{'text': rtext(r)}]
    elif form == 3 and t == 'plan':
        item['text'] = rplan(r)
    if t == 'agentMessage' and r.random() < 0.3:
        item['phase'] = r.choice(['commentary', 'final_answer', 'other', 5])
    return item if r.random() > 0.04 else r.choice([5, None, 'str', [1]])


def rturn(r):
    turn = {'items': [ritem(r) for _ in range(r.choice([0, 1, 2, 3, 3, 4, 6, 9]))]}
    with_time(r, turn, 'startedAt')
    with_time(r, turn, 'completedAt')
    return turn if r.random() > 0.03 else r.choice([5, None, 'x', []])


def rcursor(r):
    return r.choice(['c1', 'cur-2', 'c"q', 'c\\b', 'é', '😀', 'a b', ' ', 'x' * 50, 5, '', None, True])


def noise(r):
    k = r.randrange(9)
    return [None, '', 'garbage', '[1,2]', {'method': 'thread/started', 'params': {}}, {'id': 999, 'result': {}}, {'id': r.randrange(100, 200), 'method': 'srv/req'},
            {'id': 'str', 'method': 'srv/req2', 'params': {}}, {'id': None, 'method': 'srv/null'}][k]


def rpre(r):
    out = []
    while r.random() < 0.25 and len(out) < 4:
        m = noise(r)
        if m is not None:
            out.append(m)
    return out


def answer(res, pre):
    """A result action. The fake reads the client's replies to server requests in `pre` before answering, so a client
    that closes right after the response cannot race the transcript log."""
    act = {'result': res, 'pre': pre}
    n = sum(1 for m in pre if isinstance(m, dict) and 'id' in m and 'method' in m)
    if n:
        act['await'] = n
    return act


def rerror(r):
    return r.choice([
        {'code': -32601, 'message': 'Method not found'}, {'code': -32601, 'message': 'thread not found'}, {'code': -1, 'message': 'no rollout found'},
        {'code': -32000, 'message': 'Unknown thread'}, {'code': -1, 'message': 'boom'}, {'message': 'only message'}, {'code': 5}, {}, {'code': -1, 'message': 5},
        {'code': -1, 'message': 'x' * 295 + ' not found'}, {'code': -1, 'message': 'x' * 290 + ' not found'}, {'code': -1, 'message': 'NOT FOUND'},
        {'code': '-32601', 'message': 'str code'}])


def lentry(r):
    k = r.randrange(14)
    if k == 0:
        return r.choice([5, None, 'x', [], {}])
    e = {'id': r.choice(['a1', 'b-2', 'c_3', '../x', '-bad', '', 'x' * 128, 'x' * 129, 7, 'é', 'ok' + str(r.randrange(1000))])}
    if r.random() < 0.7:
        e['name'] = r.choice(TEXTS + ['n' * 300, 5, None])
    if r.random() < 0.5:
        e['preview'] = r.choice(TEXTS + [5])
    with_time(r, e, 'updatedAt')
    if r.random() < 0.4:
        e['path'] = r.choice(['/r/x.jsonl', '', 5, '/데이터/' + str(r.randrange(100))])
    if r.random() < 0.5:
        e['originator'] = r.choice(['lair', 'lair-desktop', 'LAIR', 'codex_cli', '', 5, None, 'xlair'])
    return e


def faults(r, case, methods):
    """Small chance of a transport fault on a random method; faults never race a response."""
    k = r.random()
    if k < 0.02:
        case['init'] = [{'error': {'code': -1, 'message': 'denied'}}]
    elif k < 0.035:
        case['init'] = [{'stall': True}]
        case['timeout'] = 300
    elif k < 0.045:
        case['init'] = [{'exit': 2}]
    elif k < 0.05:
        case['binary'] = '/nonexistent/codex-q'
    return k


def random_case(r, i):
    mode = 'list' if i % 3 == 0 else 'read'
    case = {'name': f'rand:{i}', 'mode': mode}
    if r.random() < 0.1:
        case['folder'] = r.choice(['/work/proj/', '/', '//', 'rel/dir', '/tmp/a "b" \\c', '/한글', ''])
    if mode == 'list':
        entries = [lentry(r) for _ in range(r.choice([0, 1, 2, 3, 5, 10, 55]))]
        shape = r.randrange(8)
        res = {'data': entries} if shape < 4 else {'threads': entries} if shape == 4 else {'data': entries, 'threads': [lentry(r)]} if shape == 5 else r.choice([{}, 5, None, {'data': {}}])
        act = answer(res, rpre(r))
        k = r.random()
        if k < 0.04:
            act = {'error': rerror(r)}
        elif k < 0.06:
            act = {'exit': 3}
        elif k < 0.075:
            act = {'stall': True}
            case['timeout'] = 300
        case['list_actions'] = [act]
        faults(r, case, 'list')
        return case
    case['thread'] = r.choice(THREADS) if r.random() < 0.1 else 't-1'
    meta = {'id': 't-1'}
    cwd = '/work/proj' if r.random() < 0.8 else r.choice(CWDS + [5, None])
    if cwd is not None:
        meta['cwd'] = cwd
    if r.random() < 0.5:
        meta['model'] = r.choice(['gpt-5', {'id': 'o3'}, '  sp  ', 'x' * 200, 'x' * 201, {'id': 5}, 5, '', '😀' * 100, '😀' * 101])
    if r.random() < 0.5:
        meta['name'] = r.choice(TEXTS + ['n' * 300, 5])
    ract = answer({'thread': meta if r.random() > 0.03 else r.choice([5, None, {}])}, rpre(r))
    k = r.random()
    if k < 0.03:
        ract = {'error': rerror(r)}
    elif k < 0.04:
        ract = {'exit': 3}
    case['read_actions'] = [ract]
    pages = []
    for p in range(r.choice([1, 1, 1, 2, 2, 3, 5, 8])):
        data = [rturn(r) for _ in range(r.choice([0, 1, 2, 3, 5, 8]))]
        res = {'data': data}
        if r.random() < 0.75:
            res['nextCursor'] = rcursor(r)
        if r.random() < 0.02:
            res = r.choice([5, None, {'data': {}}])
        act = answer(res, rpre(r))
        k = r.random()
        if k < 0.04:
            act = {'error': rerror(r)}
        elif k < 0.05:
            act = {'exit': 3}
        elif k < 0.06:
            act = {'stall': True}
            case['timeout'] = 300
        pages.append(act)
    case['pages_actions'] = pages
    faults(r, case, 'read')
    return case


def big_budget_cases(r):
    """A few heavy cases (turn budget, byte budget, page cap) with random text sizes."""
    out = []
    n = r.randrange(190, 230)
    out.append({'name': 'rand-heavy:turns', 'mode': 'read', 'pages_actions': {'$list': {'result': {'data': {'$list': {'items': [{'type': 'userMessage', 'text': 'p{i}'}, {'type': 'agentMessage', 'text': 'r{i}'}, {'type': 'commandExecution'}]}, 'n': 25, 'base': 0}, 'nextCursor': 'c{i}'}}, 'n': max(8, n // 25 + 1)}})
    out.append({'name': 'rand-heavy:bytes', 'mode': 'read', 'pages': [{'data': {'$list': {'items': [{'type': 'userMessage', 'text': {'$str': 'x', 'n': r.randrange(100000, 700000)}}, {'type': 'agentMessage', 'text': {'$str': 'y', 'n': r.randrange(1, 60000)}}]}, 'n': r.randrange(3, 9)}}]})
    out.append({'name': 'rand-heavy:pages', 'mode': 'read', 'pages_actions': {'$list': {'result': {'data': [{'items': [{'type': 'userMessage', 'text': 'p{i}'}]}], 'nextCursor': 'c{i}'}}, 'n': 45}})
    return out


def probes():
    """Fixed probes for the named policies (label prefix probe-Pn) and a few edge behaviours."""
    ok_list = {'data': [{'id': 'a', 'name': 'A'}]}
    return [
        # P1: an invalid UTF-8 line is skipped by Pruv; TS decodes it with U+FFFD and answers
        {'name': 'probe-P1:list', 'mode': 'list', 'timeout': 400, 'list_actions': [{'rawhex': b'{"id":2,"result":{"data":[{"id":"a","name":"\xff"}]}}'.hex()}]},
        # P2: lone surrogates become U+FFFD
        {'name': 'probe-P2:title', 'mode': 'list', 'list': {'data': [{'id': 'a', 'name': 'x\ud800y', 'path': '/p\udfff'}]}},
        {'name': 'probe-P2:prompt', 'mode': 'read', 'pages': [{'data': [{'items': [{'type': 'userMessage', 'text': 'a\ud83dz'}, {'type': 'agentMessage', 'text': '\ude00'}]}]}]},
        # P3: TS strips a genuine trailing U+FFFD at the byte clip
        {'name': 'probe-P3:prompt', 'mode': 'read', 'pages': [{'data': [{'items': [{'type': 'userMessage', 'text': 'a' * 32765 + '�' + 'zz'}]}]}]},
        {'name': 'probe-P3:reply', 'mode': 'read', 'pages': [{'data': [{'items': [{'type': 'userMessage', 'text': 'u'}, {'type': 'agentMessage', 'text': 'a' * 32765 + '�' + 'zz'}]}]}]},
        # P4: only integer and canonical-ISO timestamps are read
        {'name': 'probe-P4:float', 'mode': 'list', 'list': {'data': [{'id': 'a', 'updatedAt': 1.5}, {'id': 'b', 'updatedAt': '2024-01-02T03:04:05Z'}, {'id': 'c', 'updatedAt': '2024-01-02'}]}},
        {'name': 'probe-P4:turn', 'mode': 'read', 'pages': [{'data': [{'startedAt': 1700000000.25, 'completedAt': '2023-11-14T22:13:25Z', 'items': [{'type': 'userMessage', 'text': 'u'}, {'type': 'agentMessage', 'text': 'r'}, {'type': 'plan', 'text': '- a'}]}]}]},
        # P6: a response id counts only when its token is exactly the decimal id
        {'name': 'probe-P6:float-id', 'mode': 'list', 'timeout': 400, 'list_actions': [{'idtoken': '2.0', 'result': ok_list}]},
        {'name': 'probe-P6:exp-id', 'mode': 'list', 'timeout': 400, 'list_actions': [{'idtoken': '2e0', 'result': ok_list}]},
        # P8: the 200-unit plan slice drops a split surrogate pair
        {'name': 'probe-P8:plan', 'mode': 'read', 'pages': [{'data': [{'items': [{'type': 'userMessage', 'text': 'u'}, {'type': 'plan', 'text': 'x' * 199 + '😀' + '\n- ' + 'y' * 198 + '😀z'}]}]}]},
        # edge behaviours that must match exactly
        {'name': 'edge:empty-lines', 'mode': 'list', 'list_actions': [{'pre': ['', '\r', '   '], 'result': ok_list}]},
        {'name': 'edge:crlf-line', 'mode': 'list', 'list_actions': [{'raw': '{"id":2,"result":{"data":[{"id":"a","name":"A"}]}}\r'}]},
        {'name': 'edge:id-spaces', 'mode': 'list', 'list_actions': [{'raw': '  {"id":2 , "result":{"data":[{"id":"a","name":"A"}]}}  '}]},
        {'name': 'edge:dup-keys', 'mode': 'list', 'list_actions': [{'raw': '{"id":2,"result":{"data":[]},"result":{"data":[{"id":"d","name":"D"}]}}'}]},
        {'name': 'edge:error-and-result', 'mode': 'list', 'list_actions': [{'raw': '{"id":2,"result":{"data":[]},"error":{"message":"x"}}'}]},
        {'name': 'edge:error-null', 'mode': 'list', 'list_actions': [{'raw': '{"id":2,"error":null,"result":{"data":[{"id":"a","name":"A"}]}}'}]},
        {'name': 'edge:error-string', 'mode': 'list', 'list_actions': [{'raw': '{"id":2,"error":"bad","result":{"data":[{"id":"a","name":"A"}]}}'}]},
        {'name': 'edge:method-and-id', 'mode': 'list', 'list_actions': [{'raw': '{"id":2,"method":"x","result":{"data":[]}}', 'post': []}], 'timeout': 400},
        {'name': 'edge:partial-line', 'mode': 'list', 'timeout': 400, 'list_actions': [{'oversize': 100}]},
        {'name': 'edge:id-negative-zero', 'mode': 'list', 'timeout': 400, 'list_actions': [{'idtoken': '-0', 'result': ok_list}]},
    ]


def main():
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 2000
    r = random.Random(SEED)
    binary = fake_binary()
    cases = probes() + [random_case(r, i) for i in range(n)] + big_budget_cases(r)
    (WORK / 'q_cases.json').write_text(json.dumps(cases))
    report = {}
    started = time.monotonic()
    ts_requests = [request_for(case, 'qts', i, binary) for i, case in enumerate(cases)]
    reference = run_lines(node_command(), ts_requests, timeout=max(600, n * 4))
    ts_logs = [read_log(i, 'qts') for i in range(len(cases))]
    (WORK / 'q_reference.json').write_text(json.dumps(reference))
    print('reference', len(reference), round(time.monotonic() - started, 2), flush=True)
    bad_any = False
    for compiler, backend in PAIRS:
        key = f'{compiler}/{backend}'
        exe = WORK / f'{compiler}_{backend}' / 'runner'
        if not exe.exists():
            print(f'{key}: no runner (run test_impl.py first)', flush=True)
            bad_any = True
            continue
        t0 = time.monotonic()
        side = f'q{compiler}_{backend}'
        requests = [request_for(case, side, i, binary) for i, case in enumerate(cases)]
        actual = run_lines([exe], requests, timeout=max(600, n * 4))
        seconds = time.monotonic() - t0
        classes = collections.Counter()
        bad = []
        for i, case in enumerate(cases):
            label = case['name']
            cls = classify(label, actual[i], reference[i], case)
            log = read_log(i, side)
            if violations_of(log):
                cls = 'unexplained'
            # request transcripts must match unless a P1 / P6 probe makes TS go further than Pruv
            if cls != 'unexplained' and not label.startswith(('probe-P1', 'probe-P6')):
                if calls_of(log) != calls_of(ts_logs[i]) or [x for x in (log or []) if 'recv' in x] != [x for x in (ts_logs[i] or []) if 'recv' in x]:
                    cls = 'unexplained'
            classes[cls] += 1
            if cls == 'unexplained':
                bad.append({'case': label, 'actual': actual[i], 'expected': reference[i], 'violations': violations_of(log)})
        report[key] = {'cases': len(cases), 'classes': dict(classes), 'wall_seconds': round(seconds, 2), 'unexplained': bad[:50], 'unexplained_count': len(bad)}
        (WORK / f'{compiler}_{backend}_q.json').write_text(json.dumps(actual))
        (WORK / 'q-report.json').write_text(json.dumps(report, indent=1))
        print(key, dict(classes), round(seconds, 2), flush=True)
        bad_any |= bool(bad)
    raise SystemExit(1 if bad_any else 0)


if __name__ == '__main__':
    main()
