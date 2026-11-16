"""Q differential for the Grok import: seeded random scenarios plus the fixed fixtures, run on the pinned TypeScript (Node,
reference.mjs) and on the pairs built by test_impl.py, compared with oracle.compare (results, ACP transcript, primitive trace, final
trees). A difference is accepted only when oracle.py names the policy whose trigger is present in the case; anything else is
`unexplained` and fails the run (exit 1). Generation is deterministic for a seed: every case has its own Random(seed * 1000003 + index).

    GROK_TS_TRANSPILER=/path/to/typescript python3 bench/l3/grok/q.py --cases 2000 --seed 20261116
        [--pairs py/rust,rs/rust,...] [--out work/q-report.json] [--jobs N] [--no-fixed] [--only-families list,read,...]
        [--dump-cases FILE] [--keep]

Needs test_impl.py to have built the runners (work/<compiler>_<backend>/runner). Writes the summary JSON (default work/q-report.json).
"""
import sys
sys.dont_write_bytecode = True
import argparse
import collections
import json
import os
import random
import time
from concurrent.futures import ThreadPoolExecutor

from common import HERE, WORK, clean
import test_impl as T

SEED = 20261116
FAMILIES = [('list', 30), ('read', 30), ('copy', 16), ('small-json', 8), ('load', 12), ('caller-cleanup', 4)]
FOLDERS = ['/work', '/work', '/work', '/work/', '/a b', '/한글/é', '//', '/work/proj/x', "/a(b)!~*'.-_%"]
TEXTS = ['', ' ', 'hello', 'a\tb', 'line1\nline2', '  padded  ', '한글', '😀', 'é' * 30, 'z' * 300, 'a\r\nb', 'x', 'The quick brown fox', '- step', '{"json":"in text"}', '​z', '﻿x']
TITLES = ['T', '', '   ', 'two  spaces', 'tab\tand\nnl', 'a  b﻿c', {'$rep': 'w', 'n': 130}, {'$cat': [{'$rep': 'a', 'n': 119}, '😀z']},
          {'$cat': [{'$rep': 'b', 'n': 120}, 'c']}, '한글 제목', 5, None]
BAD_NAMES = ['a b', '.hid', '-lead', 'é', 'x' * 129, '..x']
DATE_TOKENS = ['1700000000', '1700000000000', '1700000009000', '1600000000', '"2024-01-02T03:04:05.000Z"', '"2023-06-07T08:09:10.111Z"', '"1700000001"',
               '"  1700000002  "', '"garbage"', '""', 'true', 'null', '[]', '-5', '0', '99999999999', '100000000000']
P4_TOKENS = ['1700000000.5', '1e3', '"2024-01-02T03:04:05Z"', '"2024-01-02T03:04:05+09:00"', '"2024-01-02"', '"2024-02-30T00:00:00.000Z"', '"-5"', '"1.5"']
MSGS = [None, 0, 3, 41, {'$raw': '"7"'}, -1, True, {'$raw': '1.5'}, {'$raw': '1e3'}, {'$raw': '-0'}, {'$raw': '9007199254740993'}, {'$raw': '9007199254740991'}, {'$raw': '1e400'}]
ALLOWED = ['summary.json', 'chat_history.jsonl', 'events.jsonl', 'updates.jsonl', 'prompt_context.json', 'resources_state.json', 'rewind_points.jsonl',
           'signals.json', 'usage.json', 'system_prompt.txt', 'tool_definitions.json', 'announcement_state.json', 'title_refresh_idx']
OTHER_NAMES = ['auth.json', 'session.lock', 'unknown.txt', '.DS_Store', 'credentials', 'chat_history.jsonl.bak']
CAPS = {'agentCapabilities': {'loadSession': True}}


def pick(r, xs):
    return xs[r.randrange(len(xs))]


def weighted(r, table):
    k = r.randrange(sum(w for _, w in table))
    for name, w in table:
        if k < w:
            return name
        k -= w
    return table[-1][0]


# ---- list -------------------------------------------------------------------------------------------------------------
def rdates(r, n):
    k = r.randrange(6)
    if k == 0:
        return 'asc'
    if k == 1:
        return 'desc'
    if k <= 3:
        return {'seed': r.randrange(10**6), 'missing_pct': pick(r, [0, 0, 10, 30, 60]), 'distinct': pick(r, [3, 10, 100])}
    pool = DATE_TOKENS + (P4_TOKENS if r.random() < 0.3 else [])
    return [pick(r, pool) for _ in range(n)]


def rsummary_over(r):
    over = {}
    k = r.random()
    if k < 0.05:
        over['info'] = {'id': 'Other'}
    elif k < 0.10:
        over['info'] = {'cwd': pick(r, ['/other', '/work/', {'$del': 1}, 5, '/Work'])}
    elif k < 0.12:
        over['info'] = pick(r, [{'$raw': '[1]'}, {'$raw': 'null'}, {'$del': 1}])
    if r.random() < 0.5:
        over['generated_title'] = pick(r, TITLES)
    if r.random() < 0.3:
        over['session_summary'] = pick(r, TITLES)
    m = pick(r, MSGS)
    if m is not None or r.random() < 0.1:
        over['num_chat_messages'] = m
    if r.random() < 0.15:
        over['current_model_id'] = pick(r, ['m1', '  m2  ', '', 5])
    return over


def rtree_entry(r, base, name):
    k = r.randrange(5)
    kind = ['file', 'dir', 'symlink', 'fifo', 'file'][k]
    e = {'path': '%s/%s' % (base, name), 'kind': kind}
    if kind == 'file':
        e['bytes'] = {'utf8': 'x'}
    elif kind == 'symlink':
        e['target'] = 'S00'
    return e


def gen_list(r, case):
    folder = pick(r, FOLDERS)
    case['folder'] = folder
    n = pick(r, [0, 1, 2, 3, 4, 5, 7, 8, 9, 12, 30, 52, 70])
    ids = []
    for k in range(n):
        ids.append('S%02d' % k if r.random() < 0.93 else pick(r, BAD_NAMES) + str(k))
    by_id = {sid: rsummary_over(r) for sid in ids if r.random() < 0.3}
    by_id = {k: v for k, v in by_id.items() if v}
    base = '$SRC/sessions/' + T.enc_folder(folder)
    also, tree = [], []
    for k in range(pick(r, [0, 0, 0, 1, 2, 4])):
        name = pick(r, ['F%d' % k, 'f g%d' % k, '.d%d' % k, 'T-%d' % k])
        also.append(name)
        tree.append(rtree_entry(r, base, name))
    order = ids + also
    r.shuffle(order)
    sess = {'ids': ids, 'dates': rdates(r, n), 'summary_by_id': by_id, 'also': also, 'order': order}
    gens = [{'sessions': sess}]
    case['gen'] = gens
    case['tree'] = tree
    if r.random() < 0.4 and n:
        pool = ids + ['nobody', '', '../x']
        entries = []
        for _ in range(r.randrange(1, 6)):
            sid = pick(r, pool)
            entries.append(pick(r, [sid, {'id': sid}, {'sessionId': sid}, {'session_id': sid}, {'id': '', 'sessionId': sid}, {'id': 5, 'sessionId': sid}, 5, None]))
        text = T.dump(entries if r.random() > 0.08 else pick(r, [{'id': 'S00'}, 'S00', None]))
        e = {'path': '$SRC/active_sessions.json', 'kind': 'file', 'bytes': {'utf8': text}}
        k = r.random()
        if k < 0.07:
            e['mode'] = 0
        elif k < 0.12:
            e = {'path': '$SRC/active_sessions.json', 'kind': 'symlink', 'target': 'nowhere'}
        elif k < 0.17:
            e['bytes'] = {'concat': [{'utf8': text}, {'repeat': {'utf8': ' '}, 'count': pick(r, [65536 - len(text), 65537 - len(text)])}]}
        tree.append(e)
    k = r.random()
    if k < 0.02:
        case['source'] = {'home': '$SRC/missing'}
    elif k < 0.04:
        case['schedule'] = {'rules': [{'match': {'op': 'readdir', 'path': base}, 'error': {'code': pick(r, ['EACCES', 'ENOENT', 'EIO'])}}]}
    elif k < 0.05:
        case['folder'] = pick(r, ['', '.', '..'])


# ---- history ----------------------------------------------------------------------------------------------------------
def rtext(r):
    return pick(r, TEXTS) if r.random() > 0.05 else pick(r, [{'$rep': 'y', 'n': 33000}, {'$rep': '日', 'n': 15000}, {'$cat': [{'$rep': 'a', 'n': 32765}, '�', 'zz']}])


def rstamp(r):
    k = r.random()
    if k < 0.45:
        return None
    pool = ['"2024-01-02T03:04:05.000Z"', '1700000000', '1700000000000', 'null', '"garbage"', '"1700000001"', '"2024-02-03T04:05:06.000Z"'] + (P4_TOKENS if r.random() < 0.25 else [])
    return {'$raw': pick(r, pool)}


def rcontent(r, user):
    k = r.randrange(7)
    t = rtext(r)
    if k == 0:
        return [{'text': t}, {'text': pick(r, TEXTS)}, {'text': 5}, 'junk', None]
    if k == 1:
        return [{'text': t}]
    if k == 2 and user:
        return pick(r, ['<user_query>\r\n %s \n</user_query>', '  <user_query>%s</user_query> ', '<user_query>\n\n%s\n\n</user_query>', '<user_info>%s</user_info>',
                        '<user_query></user_query>', '<user_query>', '<user_query>%s</user_query> <user_query>b</user_query>']).replace('%s', t if isinstance(t, str) else 'big')
    return t


def rentry(r):
    k = r.random()
    e = {}
    if k < 0.40:
        e = {'type': 'user', 'content': rcontent(r, True)}
        if r.random() < 0.15:
            e['synthetic_reason'] = pick(r, [None, False, 0, '', 'compaction'])
        if r.random() < 0.15:
            e['prompt_index'] = pick(r, [None, 0, False, '', 3])
    elif k < 0.80:
        e = {'type': 'assistant', 'content': rcontent(r, False)}
        if r.random() < 0.4:
            e['tool_calls'] = pick(r, [[], [1], [1, None, {}], [{}] * 5, 'x', None, [{'name': 'update_plan'}]])
    elif k < 0.86:
        return {'type': pick(r, ['system', 'tool', 'weird', 5, None])}
    elif k < 0.93:
        return {'$raw': pick(r, ['not json', '[1,2]', '', '   ', '{"type":"user"', '{"type":"assistant","type":"user","content":"dup"}', '{"type":"user","content":"x\\ud800y"}'])}
    elif k < 0.965:
        return {'$line': {'type': pick(r, ['user', 'assistant']), 'prefix': 'BIG', 'size': pick(r, [40000, 120000, 1048577, 1048578, 1100000]), 'cr': r.random() < 0.3,
                          'pre': pick(r, ['', '"synthetic_reason":"x",', '"synthetic_reason":0,']), 'extra': pick(r, ['', ',"synthetic_reason":"late"'])}}
    else:
        return {'$hex': '7b2274797065223a2275736572222c22636f6e74656e74223a2278ff79227d0a'}
    st = rstamp(r)
    if st is not None:
        e[pick(r, ['timestamp', 'timestamp', 'created_at'])] = st
    return e


def rhistory(r):
    n = pick(r, [0, 1, 2, 3, 4, 6, 8, 12, 30])
    if r.random() < 0.03:
        return [{'$repeat': [{'type': 'user', 'content': 'q'}, {'type': 'assistant', 'content': 'a'}], 'count': pick(r, [199, 200, 201, 230])}]
    items = [rentry(r) for _ in range(n)]
    if r.random() < 0.05:
        items.append({'$raw': '{"type":"user","content":"unterminated"}', '$nolf': True})
    return items


def gen_read(r, case):
    folder = pick(r, FOLDERS)
    thread = 'A' if r.random() < 0.94 else pick(r, ['../x', '', '-lead', 'a b', 'B'])
    case.update(folder=folder, thread=thread)
    over = rsummary_over(r) if r.random() < 0.4 else {}
    sess = {'thread': 'A', 'summary': over}
    k = r.random()
    if k < 0.04:
        pass                                          # no history file: not-found
    elif k < 0.06:
        sess['files'] = {'chat_history.jsonl': pick(r, [{'kind': 'dir'}, {'kind': 'symlink', 'target': 'summary.json'}, {'kind': 'fifo'}])}
    else:
        sess['history'] = rhistory(r)
    case['gen'] = [{'session': sess}]
    base = '$SRC/sessions/%s/A' % T.enc_folder(folder)
    k = r.random()
    if k < 0.03:
        case['schedule'] = {'rules': [{'match': {'op': pick(r, ['open', 'read', 'fstat']), 'path': base + '/chat_history.jsonl'}, 'error': {'code': pick(r, ['EACCES', 'ENOENT', 'EIO'])}}]}
    elif k < 0.05:
        case['source'] = {'home': '$SRC/missing'}


# ---- ACP --------------------------------------------------------------------------------------------------------------
def acp_scenario(r, case):
    k = r.randrange(26)
    acp = {}
    if k <= 6:
        return
    if k == 7:
        acp['initialize'] = {'result': {'agentCapabilities': pick(r, [{'loadSession': False}, {'loadSession': 'true'}, {}, {'loadSession': 1}, [True], None])}}
    elif k == 8:
        acp['initialize'] = {'result': pick(r, [None, {}, 5, 'x'])}
    elif k == 9:
        acp['initialize'] = {'error': pick(r, [{'code': -1, 'message': 'denied'}, {'code': None, 'message': 'x'}, {'code': 'E', 'message': 'x'}, {'code': {'a': [1, 2.5]}, 'message': 'x'},
                                              {'code': [1, 'a'], 'message': 'm'}, {'message': 'no code'}, {'code': 1, 'message': {'$rep': 'm', 'n': 400}}, {'code': 1, 'message': 5}])}
    elif k == 10:
        acp['session/load'] = {'error': pick(r, [{'code': -32602, 'message': 'nope'}, {'message': 'x'}, {'code': 7}])}
    elif k == 11:
        acp['session/load'] = {'omit_result': True} if r.random() < 0.5 else {'result': pick(r, [None, 5, [1, {'x': 1e3}], 'ok'])}
    elif k == 12:
        n = r.randrange(1, 4)
        pre = [{'method': 'x/notify', 'params': {}}]
        for j in range(n):
            pre.append(pick(r, [{'id': 7 + j, 'method': 'srv/req', 'params': {}}, {'id': None, 'method': 'srv/null'}, {'id': 's-%d' % j, 'method': 'srv/str'}]))
        r.shuffle(pre)
        acp['initialize'] = {'result': CAPS, 'pre': pre, 'await': sum(1 for m in pre if 'id' in m and 'method' in m)}
    elif k == 13:
        acp['initialize'] = {'id': 99, 'result': CAPS, 'also_correct': True}
    elif k == 14:
        acp['initialize'] = {'result': CAPS, 'pre': [pick(r, ['', 'garbage', '[1,2]', '   ', 'null', '{"x":', '"s"', '{"method":"n"}']) for _ in range(r.randrange(1, 5))]}
    elif k == 15:
        acp['initialize'] = {'result': CAPS, 'stderr_flood': pick(r, [1000, 100000, 300000])}
    elif k == 16:
        case['binary'] = '/nonexistent/grok-q'
    elif k == 17:
        acp[pick(r, ['initialize', 'session/load'])] = {'exit': pick(r, [0, 2, 3])}
    elif k == 18:
        acp[pick(r, ['initialize', 'session/load'])] = {'stall': True}
        case['rpcMs'] = 3000
    elif k == 19:
        acp['initialize'] = {'oversize': 100}
        case['rpcMs'] = 3000
    elif k == 20:
        acp['initialize'] = {'idtoken': pick(r, ['1.0', '1e0']), 'result': CAPS}
        case['rpcMs'] = 3000
    elif k == 21:
        tok = pick(r, ['1e400', '9007199254740993', '1.0', '12'])
        acp['initialize'] = {'result': CAPS, 'await': 1, 'pre': [{'$raw': '{"jsonrpc":"2.0","id":%s,"method":"srv/req"}' % tok, '$reply_token': tok}]}
    elif k == 22:
        acp['initialize'] = {'raw': '{"jsonrpc":"2.0","id":1,"error":{"code":%s,"message":"x"}}' % pick(r, ['-1e400', '9007199254740993', '[1.0,0.1e1]', '1e5', 'null'])}
    elif k == 23:
        acp['initialize'] = {'raw': '{"jsonrpc":"2.0","id":1,"error":"bad"}'}
        case['rpcMs'] = 3000
    elif k == 24:
        acp['initialize'] = {'result': CAPS, 'together': [{'id': 2, 'result': None}], 'p6': True}
        acp['session/load'] = {'stall': True}
        case['rpcMs'] = 3000
    else:
        acp['initialize'] = {'rawhex': '7b226a736f6e727063223a22322e30222c226964223a312c22726573756c74223a7b226167656e7443617061626c6974696573223a7b226c6f616453657373696f6e223a747275657d2c2278223a22ff227d7d'}
        case['rpcMs'] = 3000
    if acp:
        case['acp'] = acp


# ---- copy / load / small-json / cleanup ---------------------------------------------------------------------------------
def rbytes(r):
    k = r.random()
    if k < 0.10:
        return ''
    if k < 0.80:
        return pick(r, ['{}', '{"a":1}\n', 'line\n' * 5, 'x', 'é한글'])
    return {'repeat': {'utf8': pick(r, ['0123456789abcdef\n', 'x'])}, 'count': pick(r, [65535, 65536, 65537, 100000, 262144, 262145, 300001])}


def gen_copy(r, case):
    entropy = bytes(r.randrange(256) for _ in range(6)).hex()
    case.update(umask=pick(r, [18, 18, 63, 0]), schedule={'entropyHex': [entropy]})
    files = {}
    for name in r.sample(ALLOWED[2:], r.randrange(0, 8)):
        files[name] = rbytes(r)
    for name in r.sample(OTHER_NAMES, r.randrange(0, 3)):
        files[name] = rbytes(r)
    if r.random() < 0.2 and files:
        name = pick(r, sorted(files))
        files[name] = pick(r, [{'kind': 'symlink', 'target': 'summary.json'}, {'kind': 'fifo'}, {'kind': 'dir'}])
    sess = {'thread': 'A', 'readdir': True, 'files': files}
    if r.random() < 0.15:
        sess['no_summary'] = True
    if r.random() < 0.3:
        sess['history'] = rhistory(r)[:6]
    case['gen'] = [{'session': sess}]
    tree = []
    target = '$DST/sessions/%2F/A'
    temp = target + '.lair-import-' + entropy
    rules = []
    k = r.random()
    if k < 0.10:
        kind = pick(r, ['file', 'dir', 'symlink'])
        tree.append({'path': target, 'kind': kind, **({'target': 'nowhere'} if kind == 'symlink' else {})})
    elif k < 0.14:
        rules.append({'match': {'op': 'lstat', 'path': target, 'occurrence': 2, 'barrier': 'before-call'},
                      'actions': [pick(r, [{'kind': 'mkdir', 'path': target, 'mode': 448}, {'kind': 'replace-file', 'path': target, 'mode': 420, 'bytes': {'utf8': 'race'}}])]})
    elif k < 0.17:
        tree.append({'path': temp + '/old.txt', 'kind': 'file', 'bytes': {'utf8': 'old'}})
    elif k < 0.30:
        op, path = pick(r, [('mkdir', '$DST/sessions/%2F'), ('mkdir', temp), ('rename', temp), ('readdir', '$SRC/sessions/%2F/A'), ('lstat', target), ('create', temp + '/summary.json'),
                            ('write', temp + '/summary.json'), ('read', '$SRC/sessions/%2F/A/summary.json'), ('rm', temp)])
        rule = {'match': {'op': op, 'path': path}, 'error': {'code': pick(r, ['EACCES', 'EIO', 'ENOSPC', 'ENOENT'])}}
        if op == 'lstat':
            rule['match']['occurrence'] = pick(r, [1, 2])
        rules.append(rule)
        if op == 'rename' and r.random() < 0.5:
            rules.append({'match': {'op': 'rm', 'path': temp}, 'error': {'code': 'EACCES'}})
    elif k < 0.34:
        case['source'] = {'engine': pick(r, ['codex', 'grok']), 'home': '$SRC'}
        if r.random() < 0.5:
            case['thread'] = pick(r, ['../A', '', 'B'])
        if r.random() < 0.3:
            case['targetHome'] = ''
    if rules:
        case['schedule']['rules'] = rules
    if tree:
        case['tree'] = tree
    acp_scenario(r, case)
    if r.random() < 0.08:                              # a failed target cleanup after a failed verify must not change the result
        case['schedule'].setdefault('rules', []).append({'match': {'op': 'rm', 'path': target}, 'error': {'code': 'EACCES'}})


def gen_load(r, case):
    acp_scenario(r, case)
    if r.random() < 0.05:
        case['thread'] = pick(r, ['B', 'x' * 128])


def rjson(r, depth=0):
    k = r.randrange(9 if depth < 4 else 6)
    if k == 0:
        return pick(r, [{'$raw': t} for t in ['1', '1.0', '1e2', '9007199254740993', '-0', '0.1', '1E+2', '-1.5e-3', '123456789012345678901234567890', '1e400', '-1e400', '1e-400']])
    if k == 1:
        return pick(r, ['', 'x', 'é', '😀', 'a b', {'$raw': '"\\ud800"'}, {'$raw': '"\\u00e9\\ud83d\\ude00"'}])
    if k == 2:
        return pick(r, [True, False, None])
    if k == 3:
        return r.randrange(-1000, 100000)
    if k in (4, 5):
        return {'$raw': '"%s"' % pick(r, ['abc', 'a\\nb', 'a\\"b', '\\u0000', '\\ud800\\udc00'])}
    if k <= 7:
        return [rjson(r, depth + 1) for _ in range(r.randrange(0, 5))]
    keys = ['a', 'b', 'k\\ud800', 'é', 'a']      # a repeated key: the last one wins
    return {'$raw': '{%s}' % ','.join('"%s":%s' % (pick(r, keys), T.dump(rjson(r, depth + 1))) for _ in range(r.randrange(0, 5)))}


def gen_small(r, case):
    k = r.random()
    if k < 0.07:
        body = {'utf8': pick(r, ['', '[1,2]', 'null', '"x"', '{"a":', '{', '5', 'true', '{"a":1}{"b":2}'])}
    elif k < 0.10:
        body = {'concat': [{'utf8': '{"a":"'}, {'repeat': {'utf8': 'x'}, 'count': pick(r, [65528, 65529])}, {'utf8': '"}'}]}
    elif k < 0.13:
        body = {'concat': [{'utf8': '{"a":"x'}, {'hex': 'ff'}, {'utf8': 'y"}'}]}
    elif k < 0.16:
        body = {'concat': [{'utf8': '{"a":'}, {'repeat': {'utf8': '['}, 'count': 129}, {'repeat': {'utf8': ']'}, 'count': 129}, {'utf8': '}'}]}
    else:
        body = {'utf8': T.dump({'$raw': '{%s}' % ','.join('"%s":%s' % (pick(r, ['a', 'b', 'c']), T.dump(rjson(r))) for _ in range(r.randrange(0, 5)))})}
    path = pick(r, ['$SRC/x.json'] * 6 + ['$SRC/missing.json'])
    tree = [{'path': '$SRC/x.json', 'kind': pick(r, ['file'] * 9 + ['dir', 'fifo']), 'bytes': body}]
    case.update(smallPath=path, tree=tree)
    if r.random() < 0.04:
        case['schedule'] = {'rules': [{'match': {'op': pick(r, ['open', 'read', 'lstat']), 'path': '$SRC/x.json'}, 'error': {'code': pick(r, ['EACCES', 'EIO'])}}]}
    if r.random() < 0.04:
        case['smallPath'] = '$SRC/h2/x.json'
        case['smallHomeReal'] = '$SRC_REAL/h'
        case['tree'] = [{'path': '$SRC/h', 'kind': 'dir'}, {'path': '$SRC/h2/x.json', 'kind': 'file', 'bytes': {'utf8': '{"a":1}'}}]


def gen_cleanup(r, case):
    path = pick(r, ['$DST/sessions/%2F/A', '$DST/sessions/%2F/missing'])
    case['cleanupPath'] = path
    if r.random() < 0.8:
        case['tree'] = [{'path': '$DST/sessions/%2F/A/summary.json', 'kind': 'file', 'bytes': {'utf8': '{}'}}, {'path': '$DST/sessions/%2F/A/sub/x', 'kind': 'file', 'bytes': {'utf8': 'x'}},
                        {'path': '$SRC/sessions/%2F/A/summary.json', 'kind': 'file', 'bytes': {'utf8': '{}'}}]


GENERATORS = {'list': gen_list, 'read': gen_read, 'copy': gen_copy, 'load': gen_load, 'small-json': gen_small, 'caller-cleanup': gen_cleanup}


def random_case(seed, i, families=None):
    r = random.Random(seed * 1000003 + i)
    table = [(f, w) for f, w in FAMILIES if not families or f in families]
    mode = weighted(r, table)
    case = {'name': 'q%05d-%s' % (i, mode), 'mode': mode, 'category': 'random'}
    GENERATORS[mode](r, case)
    return T.expand_case(case, json.loads((T.FIXTURES / '_defaults.json').read_text(encoding='utf-8')))


# ---- running ----------------------------------------------------------------------------------------------------------
def classify(v):
    if not v['ok']:
        return 'unexplained'
    if v['policies']:
        return 'policy:' + '+'.join(v['policies'])
    return 'note' if v['notes'] else 'exact'


def run_one(case, exes, keep, expected_check):
    """One case on the pinned TypeScript and on every pair: {pair: verdict}, plus fixture-bug problems."""
    t0 = time.monotonic()
    ts, base = T.run_reference(case)
    problems = T.check_expected(case, ts) if expected_check else []
    out, impls = {}, []
    for (compiler, backend), exe in exes.items():
        impl = T.run_pair(case, '%s_%s' % (compiler, backend), exe)
        v = T.judge(case, ts, base, impl)
        out['%s/%s' % (compiler, backend)] = v
        impls.append(impl)
    clean_all = not problems and all(v['ok'] for v in out.values())
    if clean_all and not keep:
        for side in [ts] + impls:
            if side.get('root'):
                clean(side['root'])
    return case['name'], out, problems, time.monotonic() - t0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--cases', type=int, default=2000, help='number of random cases (default 2000)')
    ap.add_argument('--seed', type=int, default=SEED)
    ap.add_argument('--pairs', help='comma list (default: every pair whose runner exists in work/)')
    ap.add_argument('--out', default=str(WORK / 'q-report.json'))
    ap.add_argument('--jobs', type=int, default=4)
    ap.add_argument('--no-fixed', action='store_true', help='random cases only')
    ap.add_argument('--only-families', help='comma list of list,read,copy,small-json,load,caller-cleanup')
    ap.add_argument('--dump-cases', help='write the expanded case list as JSON and exit (checks determinism)')
    ap.add_argument('--keep', action='store_true')
    ap.add_argument('--ts-transpiler')
    args = ap.parse_args(argv)
    families = set(args.only_families.split(',')) if args.only_families else None
    random_cases = [random_case(args.seed, i, families) for i in range(args.cases)]
    fixed, _ = ([], []) if args.no_fixed else T.load_fixtures()
    cases = fixed + random_cases
    if args.dump_cases:
        with open(args.dump_cases, 'w', encoding='utf-8') as f:
            json.dump(cases, f, indent=1, ensure_ascii=False, default=str)
        print('%d cases written (%d fixed + %d random, seed %d)' % (len(cases), len(fixed), len(random_cases), args.seed))
        return 0
    if args.ts_transpiler:
        os.environ['GROK_TS_TRANSPILER'] = args.ts_transpiler
    if not os.environ.get('GROK_TS_TRANSPILER'):
        print('GROK_TS_TRANSPILER (or --ts-transpiler) must name the typescript package', file=sys.stderr)
        return 2
    c, g, run = T.configure()
    exes, missing = {}, []
    for compiler, backend in T.parse_pairs(args.pairs):
        exe = WORK / ('%s_%s' % (compiler, backend)) / 'runner'
        if exe.exists():
            exes[(compiler, backend)] = exe
        else:
            missing.append('%s/%s' % (compiler, backend))
    if missing:
        print('no runner for %s (run test_impl.py first)' % ', '.join(missing), file=sys.stderr)
        if not exes or args.pairs:
            return 2
    fixed_names = {c['name'] for c in fixed}
    t0 = time.monotonic()
    report = {'seed': args.seed, 'random_cases': len(random_cases), 'fixed_cases': len(fixed), 'pairs': {}, 'unexplained': [], 'fixture_problems': [], 'policies_observed': {}}
    stats = {'%s/%s' % p: collections.Counter() for p in exes}
    seconds = collections.Counter()
    seen = collections.defaultdict(set)
    with ThreadPoolExecutor(args.jobs) as ex:
        futures = [ex.submit(run_one, case, exes, args.keep, case['name'] in fixed_names) for case in cases]
        for n, fut in enumerate(futures, 1):
            name, verdicts, problems, secs = fut.result()
            for key, v in verdicts.items():
                cls = classify(v)
                stats[key][cls] += 1
                for p in v['policies']:
                    seen[p].add(name)
                if cls == 'unexplained':
                    report['unexplained'].append({'case': name, 'pair': key, 'problems': v['problems'][:3]})
            seconds['all'] += secs
            if problems:
                report['fixture_problems'].append({'case': name, 'problems': problems})
            if n % 100 == 0 or n == len(cases):
                bad = sum(s['unexplained'] for s in stats.values())
                print('%d/%d cases, %d unexplained, %.0f s' % (n, len(cases), bad, time.monotonic() - t0), flush=True)
    for key, cnt in stats.items():
        report['pairs'][key] = dict(cnt)
        print(key, dict(cnt), flush=True)
    report['policies_observed'] = {p: len(v) for p, v in sorted(seen.items())}
    report['wall_seconds'] = round(time.monotonic() - t0, 1)
    report['unexplained'] = report['unexplained'][:400]
    report['unexplained_count'] = sum(s['unexplained'] for s in stats.values())
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, 'w', encoding='utf-8') as f:
        json.dump(report, f, indent=1, default=str)
    print('policies observed:', report['policies_observed'])
    print('summary written to', args.out)
    bad = report['unexplained_count'] + len(report['fixture_problems'])
    print('FAILED: %d unexplained, %d fixture problems' % (report['unexplained_count'], len(report['fixture_problems'])) if bad else 'no unexplained differences')
    return 1 if bad else 0


if __name__ == '__main__':
    raise SystemExit(main())
