"""Q differential for the OpenCode import: a seeded generator plus the fixed fixtures, run on the pinned TypeScript (Node, reference.mjs)
and on every compiler/backend pair built by test_impl.py, compared with oracle.compare (result, SQL/FS/child trace, fake-exporter
transcript, final trees). A difference passes only through a policy whose trigger oracle.py detects in the case inputs; anything else
is `unexplained` and fails the run (exit 1). Every case has its own Random(seed * 1000003 + index): a seed gives the same cases always.

    GROK_TS_TRANSPILER=/path/to/typescript python3 bench/l3/opencode/q.py --cases 2000 --seed 20261117
        [--pairs py/rust,rs/rust,...] [--out work/q-report.json] [--policy-report work/q-policies.json] [--jobs N] [--no-fixed]
        [--only-families list,read,rows,dataDir,folderKey,plan] [--dump-cases FILE] [--keep]

Needs test_impl.py to have built the runners (work/<compiler>_<backend>/runner). Writes two reports into work/:
  q-report.json      per pair: counts by class exact | policy:Pn | note | unexplained; the first unexplained cases; cross-pair disagreements;
                     fixed-fixture `expected` problems
  q-policies.json    per pair and policy: the cases explained by it (explained deviations), separate from unexplained failures and from
                     excluded-domain observations (severity note without a policy)
Only the list clock is frozen (nowMs). Timer-sensitive cases (real stalled children) run serially; stalls use the same shorter rpcMs on both
sides, the grace periods 1.5 s / 1 s stay real. The real 20 s probe is the fixed slow fixture F08-real-20000ms (test_impl.py --slow).
"""
import sys
sys.dont_write_bytecode = True
import argparse
import collections
import json
import os
import random
import struct
import time
from concurrent.futures import ThreadPoolExecutor

from common import HERE, WORK, clean, set_transpiler

SEED = 20261117
FAMILIES = [('list', 26), ('read', 44), ('rows', 8), ('dataDir', 6), ('folderKey', 6), ('plan', 10)]
NOW = 1700000000000
FOLDERS = ['/work', '/work', '/work', '/work/', '/work//', '/', '//', '/한글/é', '/a b/c', '/work/sub']
TITLES = ['T', '', '  ', 'two  spaces', 'tab\tand\nnl', 'New session - 2026', 'Child session - x', ' New session - lead', 'a  b c﻿d',
          {'$rep': 'w', 'n': 130}, {'$cat': [{'$rep': 'a', 'n': 119}, '😀z']}, {'$cat': [{'$rep': 'b', 'n': 120}, 'c']}, '한글 제목', None, 5]
TEXTS = ['', ' ', 'hello', 'a\tb', 'line1\nline2', '  padded  ', '한글', '😀', 'é' * 30, 'z' * 300, 'a\r\nb', 'The quick brown fox', '{"json":"in text"}',
         {'$cat': [{'$rep': 'x', 'n': 32760}, '😀tail']}, {'$cat': [{'$rep': 'a', 'n': 32765}, '�', 'zz']}, {'$rep': 'y', 'n': 33000}]
STAMPS = ['1700000000', '1700000000000', '1700000009000', '1600000000', '"2024-01-02T03:04:05.000Z"', '"2023-06-07T08:09:10.111Z"', '"1700000001"',
          '"  1700000002  "', '"garbage"', '""', 'null', 'true', '[]', '-5', '0', '99999999999', '100000000000']
P4_STAMPS = ['1700000000.5', '1e3', '"2024-01-02T03:04:05Z"', '"2024-01-02T03:04:05+09:00"', '"2024-01-02"', '"2024-02-30T00:00:00.000Z"', '"-5"', '"1.5"',
             '253402300800000']
STATUSES = ['pending', 'in_progress', 'completed', 'done', None, 5, '']
ROLES = ['user', 'user', 'assistant', 'assistant', 'assistant', 'assistant', 'system', 'tool', 5, None]
UPDATED = [{'kind': 'integer', 'decimal': '1700000000000'}, {'kind': 'integer', 'decimal': '1700000000'}, {'kind': 'integer', 'decimal': '0'},
           {'kind': 'integer', 'decimal': '-5'}, {'kind': 'integer', 'decimal': '1700000100000'}, {'kind': 'integer', 'decimal': '9007199254740991'},
           {'kind': 'real', 'bits': '41d9cf7e2f400000'}, {'kind': 'real', 'bits': '3ff8000000000000'}, {'kind': 'real', 'bits': '43f0000000000000'},
           {'kind': 'text', 'text': '1700000000000'}, {'kind': 'text', 'text': ' +17 '}, {'kind': 'text', 'text': '1e3'}, {'kind': 'text', 'text': '1.5'},
           {'kind': 'text', 'text': '0x10'}, {'kind': 'text', 'text': ''}, {'kind': 'text', 'text': 'x'}, {'kind': 'null'}, {'kind': 'blob', 'hex': '05'},
           {'kind': 'blob', 'hex': ''}, {'kind': 'blob', 'hex': '0102'}]
DDL_SESSION = 'CREATE TABLE session(id,parent_id,directory,title,time_updated%s)'
DDL_MESSAGE = 'CREATE TABLE message(session_id%s)'


def pick(r, xs):
    return xs[r.randrange(len(xs))]


def weighted(r, table):
    k = r.randrange(sum(w for _, w in table))
    for name, w in table:
        if k < w:
            return name
        k -= w
    return table[-1][0]


def tx(s):
    return {'kind': 'text', 'text': s}


def tnull():
    return {'kind': 'null'}


def titled(v):
    """A TITLES entry (string / macro / None / number) as a typed SQL value."""
    if v is None:
        return tnull()
    if isinstance(v, int):
        return {'kind': 'integer', 'decimal': str(v)}
    return tx(v)


# ---- databases ------------------------------------------------------------------------------------------------------------
def sid(i):
    return 'S%04d' % i


def gen_db(r, n, directory_for, *, archive=None, bad_schema=None, thread=None):
    """DDL + inserts: n sessions; `directory_for(i)` gives the typed directory; optional archive column; storage classes vary."""
    arch = archive if archive is not None else r.random() < 0.4
    has_mid = r.random() < 0.3
    full = ['id', 'parent_id', 'directory', 'title', 'time_updated'] + (['time_archived'] if arch else [])
    present = list(full)
    ddl = [DDL_SESSION % (',time_archived' if arch else ''), DDL_MESSAGE % (',id' if has_mid else '')]
    if bad_schema == 'session':
        drop = pick(r, ['id', 'parent_id', 'directory', 'title', 'time_updated'])
        present = [c for c in full if c != drop]
        ddl[0] = 'CREATE TABLE session(%s)' % ','.join(present)
    elif bad_schema == 'message':
        ddl[1] = 'CREATE TABLE message(x)'
    srows, mrows = [], []
    for i in range(n):
        ident = tx(thread if (thread is not None and i == 0) else (sid(i) if r.random() < 0.92 else pick(r, ['bad.id', '-lead', 'a b', 'x' * 129, ''])))
        if thread is None and r.random() < 0.02:
            ident = pick(r, [{'kind': 'integer', 'decimal': '7'}, tnull(), {'kind': 'blob', 'hex': '4142'}])
        parent = tnull() if (thread is not None and i == 0 and r.random() < 0.93) or r.random() < 0.9 else tx('P')
        vals = {'id': ident, 'parent_id': parent, 'directory': directory_for(i), 'title': titled(pick(r, TITLES)), 'time_updated': dict(pick(r, UPDATED)),
                'time_archived': pick(r, [tnull(), tnull(), tnull(), {'kind': 'integer', 'decimal': '0'}, {'kind': 'integer', 'decimal': '1700000000000'}])}
        srows.append([vals[c] for c in present])
        for _ in range(pick(r, [0, 1, 1, 1, 2, 4]) if r.random() > 0.05 else 0):
            mrows.append([ident if ident['kind'] == 'text' else tx(sid(i))] + ([tx('m%d' % len(mrows))] if has_mid else []))
    ins = [{'table': 'session', 'columns': present, 'rows': srows}]
    if mrows and bad_schema != 'message':
        ins.append({'table': 'message', 'columns': ['session_id'] + (['id'] if has_mid else []), 'rows': mrows})
    if r.random() < 0.3:
        r.shuffle(srows)                                   # insertion order differs from id order; SQL order then decides
    return {'path': '$SRC/opencode.db', 'ddl': ddl, 'inserts': ins}


# ---- list / rows ------------------------------------------------------------------------------------------------------------
def gen_list(r, case, rows_mode=False):
    folder = pick(r, FOLDERS)
    case['folder'] = folder
    case['nowMs'] = NOW
    n = pick(r, [0, 1, 2, 3, 5, 8, 20, 49, 50, 51, 53])
    base = folder.rstrip('/') or folder
    dirs = [base, base + '/', base + '//', base + '/x', '/other', '', base.upper()]

    def directory(i):
        if r.random() < 0.7:
            return tx(base if r.random() < 0.6 else base + '/')
        v = pick(r, dirs + [5, None])
        return tx(v) if isinstance(v, str) else (tnull() if v is None else {'kind': 'integer', 'decimal': '5'})
    bad = pick(r, [None] * 28 + ['session', 'message'])
    case['db'] = gen_db(r, n, directory, bad_schema=bad)
    if r.random() < 0.04:
        case['db']['wal'] = True
    roll = r.random()
    if roll < 0.05:
        case['tree'] = [{'path': '$TMP', 'kind': 'dir', 'mode': 448}, {'path': '$SRC/opencode.db', 'kind': pick(r, ['dir', 'fifo'])}]
        case['db'] = {}                                    # explicit empty: no database image is placed (the defaults must not add one)
    elif roll < 0.08:
        case['db']['path'] = '$SRC/other.db'
        case['env'] = {'mode': 'full', 'values': {'OPENCODE_DB': pick(r, ['other.db', ' other.db ', '$SRC/other.db', ':memory:', '', 'sub/../other.db'])}}
    elif roll < 0.10:
        cls = pick(r, ['OS', 'SQLite'])
        op = pick(r, ['db_open_readonly', 'db_close']) if cls == 'SQLite' else pick(r, ['lstat', 'realpath'])
        case['schedule'] = {'rules': [{'match': {'op': op, 'occurrence': 1}, 'error': {'class': cls, 'code': 'EIO' if cls == 'OS' else 'SQLITE_CORRUPT'}}]}
    if rows_mode:
        case['mode'] = 'rows'


# ---- read: the export document ---------------------------------------------------------------------------------------------
def stamp(r):
    k = r.random()
    if k < 0.35:
        return None
    pool = STAMPS + (P4_STAMPS if r.random() < 0.2 else [])
    return {'$raw': pick(r, pool)}


def mark(r):
    return pick(r, [True, False, 'true', 1, 0, None, 'yes'])


def todo(r):
    t = {}
    if r.random() < 0.9:
        t['content'] = pick(r, ['step one', '  padded  ', '', '   ', 'a\tb  c', {'$cat': [{'$rep': 'p', 'n': 199}, '😀']}, {'$rep': 'q', 'n': 250}, '한글', 5, None])
    if r.random() < 0.85:
        t['status'] = pick(r, STATUSES)
    return pick(r, [t, t, t, 'junk', None, 5, []])


def parts_of(r, user):
    out = []
    for _ in range(pick(r, [0, 1, 1, 2, 3, 5])):
        k = r.randrange(9)
        if k <= 3:
            p = {'type': 'text', 'text': pick(r, TEXTS) if r.random() < 0.8 else pick(r, ['', 'x'])}
            if r.random() < 0.25:
                p['synthetic'] = mark(r)
            if r.random() < 0.2:
                p['ignored'] = mark(r)
            out.append(p)
        elif k == 4:
            out.append({'type': 'text', 'text': 5})
        elif k == 5:
            state = {'status': pick(r, ['completed', 'error', 'running', None]), 'input': {'todos': [todo(r) for _ in range(pick(r, [0, 1, 2, 3, 30, 31, 34]))]
                                                                                      if r.random() < 0.9 else pick(r, ['x', None, {}])}}
            out.append({'type': 'tool', 'tool': pick(r, ['todowrite', 'todowrite', 'bash', 'read']), 'state': state if r.random() < 0.9 else pick(r, [None, 'x', []])})
        elif k == 6:
            out.append({'type': 'tool', 'tool': 'bash'})
        elif k == 7:
            out.append(pick(r, ['junk', None, 5, [], {'type': 'step-start'}, {'type': 'reasoning', 'text': 'r'}]))
        else:
            out.append({'type': 'text', 'text': pick(r, TEXTS), 'extra': {'$raw': pick(r, ['1e400', '1.0', '9007199254740993', '-0', '"\\ud800"'])}})
    return out


def gen_message(r):
    role = pick(r, ROLES)
    info = {'role': role}
    t = {}
    c, d = stamp(r), stamp(r)
    if c is not None:
        t['created'] = c
    if d is not None:
        t['completed'] = d
    if t or r.random() < 0.5:
        info['time'] = t if r.random() > 0.05 else pick(r, [None, 5, []])
    if role == 'assistant':
        if r.random() < 0.15:
            info['summary'] = pick(r, [True, True, False, 'true', 1])
        if r.random() < 0.6:
            info['providerID'] = pick(r, ['prov', ' p ', '', 5, 'x' * 40])
            info['modelID'] = pick(r, ['m1', ' m ', '', None, 'é'])
    m = {'info': info, 'parts': parts_of(r, role == 'user')}
    k = r.random()
    if k < 0.02:
        return pick(r, [None, [], 5, {}, {'info': info}, {'parts': []}, {'info': [1], 'parts': []}, {'info': info, 'parts': {}}])
    return m


def gen_export(r):
    n = pick(r, [0, 1, 2, 3, 4, 6, 10, 30, 80])
    if r.random() < 0.03:
        n = pick(r, [199, 200, 201, 230])
        msgs = []
        for i in range(n):
            msgs.append({'info': {'role': 'user', 'time': {'created': {'$raw': '1700000000'}}}, 'parts': [{'type': 'text', 'text': 'q%d' % i}]})
            msgs.append({'info': {'role': 'assistant', 'time': {'completed': {'$raw': '1700000001'}}}, 'parts': [{'type': 'text', 'text': 'a%d' % i}]})
        return {'info': {'title': 'T'}, 'messages': msgs}
    doc = {'messages': [gen_message(r) for _ in range(n)]}
    if r.random() < 0.7:
        doc['info'] = {'title': pick(r, TITLES)} if r.random() < 0.8 else pick(r, [None, 5, [], 'x'])
    k = r.random()
    if k < 0.04:
        doc['messages'] = pick(r, [None, {}, 'x', 5])
    elif k < 0.06:
        return pick(r, [[], None, 'x', 5, {'$raw': 'null'}])
    return doc


def gen_read(r, case):
    thread = 'S0000' if r.random() < 0.93 else pick(r, ['../x', '', '-lead', 'a b', 'S0001', 'x' * 129])
    folder = '/work' if r.random() < 0.8 else pick(r, ['/work/', '/work//', '/other', '/'])
    case.update(thread=thread, folder=folder)
    stored = pick(r, ['/work', '/work', '/work', '/work/', '/work//'])
    case['db'] = gen_db(r, pick(r, [1, 1, 1, 2, 3]), lambda i: tx(stored), thread='S0000', archive=r.random() < 0.2,
                        bad_schema=pick(r, [None] * 40 + ['session', 'message']))
    exp = {'expect': {'cwd': '$SRC'}}
    k = r.random()
    if k < 0.04:
        exp['bytes'] = {'concat': [{'utf8': '{"messages":[{"info":{"role":"user"},"parts":[{"type":"text","text":"x'}, {'hex': 'ff'}, {'utf8': 'y"}]}]}'}]}
    elif k < 0.07:
        exp['bytes'] = {'utf8': pick(r, ['', '{', '[1,2', 'nul', '{"messages":[]}x', '{"messages":[]}\n\n'])}
    elif k < 0.09:
        exp['bytes'] = {'concat': [{'utf8': '{"messages":[],"a":'}, {'repeat': {'utf8': '['}, 'count': pick(r, [128, 129])}, {'repeat': {'utf8': ']'}, 'count': pick(r, [128, 129])}, {'utf8': '}'}]}
    else:
        exp['bytes'] = {'json': gen_export(r)}
    k = r.random()
    if k < 0.04:
        exp.update(mode='output', exit=pick(r, [1, 2, 3, 127]))
    elif k < 0.06:
        exp.update(mode='signal', signal=pick(r, [15, 9, 1]))
    elif k < 0.085:
        exp.update(mode='hang', term=pick(r, ['exit', 'ignore']), termExit=pick(r, [0, 3]))
        case['rpcMs'] = 1000                               # a loaded host can take >300 ms to start the fake (SIGTERM before its handler)
        case['serial'] = True
    case['export'] = exp
    k = r.random()
    if k < 0.03:
        case['binary'] = pick(r, ['', None, '/nonexistent/opencode-q'])
    elif k < 0.05:
        case['tempRoot'] = pick(r, ['', None, '$TMP/not-there'])
    elif k < 0.065:
        case['env'] = {'mode': 'full', 'values': {'OPENCODE_Q': 'v', 'EMPTY': ''}}
        exp['env'] = {'OPENCODE_Q': 'v', 'EMPTY': ''}
    elif k < 0.10:
        base = '$TMP/opencode-@1/export.json'
        op = pick(r, ['mkdtemp', 'open', 'spawn', 'close', 'stat', 'read', 'rm'])
        path = {'mkdtemp': '$TMP/opencode-', 'open': base, 'spawn': '$BIN/exporter', 'close': base, 'stat': base, 'read': base, 'rm': '$TMP/opencode-@1'}[op]
        if op == 'mkdtemp':
            path = '$TMP/opencode-'
        case['schedule'] = {'rules': [{'match': {'op': op, 'path': path}, 'error': {'class': 'OS', 'code': pick(r, ['EIO', 'EACCES', 'ENOSPC'])}}]}
    elif k < 0.12:
        case['schedule'] = {'rules': [{'match': {'op': 'db_query', 'phase': pick(r, ['all', 'get', 'prepare']), 'occurrence': pick(r, [1, 2])},
                                       'error': {'class': 'SQLite', 'code': 'SQLITE_ERROR'}}]}


# ---- utilities ---------------------------------------------------------------------------------------------------------------
def gen_datadir(r, case):
    values = {}
    if r.random() < 0.85:
        values['XDG_DATA_HOME'] = pick(r, ['', ' ', '/xdg', ' /xdg ', 'rel/path', '/한글/데이터', '/', '//', '/a//b/../c', '\t/x\n', '/x/', ' /nb '])
    case['env'] = {'mode': 'full', 'values': values}
    case['homedir'] = pick(r, ['$HOME', '/home/u', '/', '/한글'])


def gen_folderkey(r, case):
    case['folder'] = pick(r, ['', '/', '//', '/a', '/a/', '/a//', '/a/b///', '/A/b', 'a\\b\\', '/a/./b/', '/한글/', ' /a/ ', '///x', {'$cat': ['/', {'$rep': 'é', 'n': 40}, '//']}])


def gen_plan(r, case):
    msgs = []
    for _ in range(pick(r, [0, 1, 2, 3, 5])):
        m = {'parts': parts_of(r, False)}
        if r.random() < 0.7:
            m['at'] = pick(r, ['2024-01-02T03:04:05.000Z', '2023-06-07T08:09:10.111Z', ''])
        msgs.append(m)
    case['planJson'] = {'$jsonText': msgs}


GENERATORS = {'list': lambda r, c: gen_list(r, c), 'rows': lambda r, c: gen_list(r, c, True), 'read': gen_read, 'dataDir': gen_datadir,
              'folderKey': gen_folderkey, 'plan': gen_plan}


def random_case(seed, i, families=None, defaults=None, T=None):
    r = random.Random(seed * 1000003 + i)
    table = [(f, w) for f, w in FAMILIES if not families or f in families]
    mode = weighted(r, table)
    case = {'name': 'q%05d-%s' % (i, mode), 'mode': mode, 'category': 'random'}
    GENERATORS[mode](r, case)
    plan = case.get('planJson')
    if isinstance(plan, dict) and '$jsonText' in plan:        # JSON text of the array, with $raw tokens preserved (test_impl.dump)
        case['planJson'] = T.dump(T.macro(plan['$jsonText']))
    return T.expand_case(case, defaults)


# ---- running ----------------------------------------------------------------------------------------------------------------
def classify(v):
    if not v['ok']:
        return 'unexplained'
    if v['policies']:
        return 'policy:' + '+'.join(v['policies'])
    return 'note' if v['notes'] else 'exact'


def agreement(case, runs, oracle):
    """All pairs must agree with each other on result, normalised trace and final trees (DESIGN section 7: all four Pruv results agree)."""
    names = list(runs)
    out = []
    first = names[0]

    def view(run):
        try:
            return (oracle.norm_result(run['result'], run['roots']), oracle.trace_view(run['trace'], run['roots']),
                    {p: oracle.tree_view(run['after'][p], oracle.temp_tags(run.get('trace'))) for p in ('src', 'home', 'tmp', 'folder')}, oracle.exporter_view(run))
        except Exception as exc:                      # a normalisation failure is reported by the per-pair comparison
            return ('view-error', str(exc))
    base = view(runs[first])
    for n in names[1:]:
        if view(runs[n]) != base:
            out.append('%s differs from %s' % (n, first))
    return out


def run_one(case, exes, keep, T, oracle):
    t0 = time.monotonic()
    ts, base = T.run_reference(case)
    problems = T.check_expected(case, ts) if case.get('expected') else []
    out, impls, runs = {}, [], {}
    for (compiler, backend), exe in exes.items():
        impl = T.run_pair(case, '%s_%s' % (compiler, backend), exe)
        out['%s/%s' % (compiler, backend)] = T.judge(case, ts, base, impl)
        runs['%s/%s' % (compiler, backend)] = impl
        impls.append(impl)
    disagree = agreement(case, runs, oracle) if len(runs) > 1 and all(T.impl_clean(x) for x in impls) else []
    clean_all = not problems and not disagree and all(v['ok'] for v in out.values())
    if clean_all and not keep:
        for side in [ts] + impls:
            if side.get('root'):
                clean(side['root'])
    return case['name'], out, problems, disagree, time.monotonic() - t0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--cases', type=int, default=2000, help='number of generated cases (default 2000)')
    ap.add_argument('--seed', type=int, default=SEED)
    ap.add_argument('--pairs', help='comma list (default: every pair whose runner exists in work/)')
    ap.add_argument('--out', default=str(WORK / 'q-report.json'))
    ap.add_argument('--policy-report', default=str(WORK / 'q-policies.json'))
    ap.add_argument('--jobs', type=int, default=4)
    ap.add_argument('--no-fixed', action='store_true', help='generated cases only')
    ap.add_argument('--only-families', help='comma list of list,read,rows,dataDir,folderKey,plan')
    ap.add_argument('--dump-cases', help='write the expanded case list as JSON and exit (checks determinism)')
    ap.add_argument('--keep', action='store_true')
    ap.add_argument('--ts-transpiler')
    args = ap.parse_args(argv)
    import test_impl as T
    import oracle
    defaults = json.loads((T.FIXTURES / '_defaults.json').read_text(encoding='utf-8'))
    families = set(args.only_families.split(',')) if args.only_families else None
    generated = [random_case(args.seed, i, families, defaults, T) for i in range(args.cases)]
    fixed = [] if args.no_fixed else T.load_fixtures()[0]
    cases = fixed + generated
    if args.dump_cases:
        with open(args.dump_cases, 'w', encoding='utf-8') as f:
            json.dump(cases, f, indent=1, ensure_ascii=False, default=str)
        print('%d cases written (%d fixed + %d generated, seed %d)' % (len(cases), len(fixed), len(generated), args.seed))
        return 0
    if args.ts_transpiler:
        set_transpiler(args.ts_transpiler)
    if not os.environ.get('GROK_TS_TRANSPILER'):
        print('GROK_TS_TRANSPILER (or --ts-transpiler) must name the typescript package', file=sys.stderr)
        return 2
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
    report = {'seed': args.seed, 'generated_cases': len(generated), 'fixed_cases': len(fixed), 'pairs': {}, 'unexplained': [], 'fixture_problems': [],
              'pair_disagreements': []}
    stats = {'%s/%s' % p: collections.Counter() for p in exes}
    explained = {k: collections.defaultdict(list) for k in stats}       # pair -> policy -> cases
    domain = {k: [] for k in stats}                                      # excluded-domain observations (notes without a policy)
    done = collections.Counter()
    parallel = [c for c in cases if not T.timer_sensitive(c)]
    serial = [c for c in cases if T.timer_sensitive(c)]

    def consume(res):
        name, verdicts, problems, disagree, _secs = res
        for key, v in verdicts.items():
            cls = classify(v)
            stats[key][cls] += 1
            for p in v['policies']:
                explained[key][p].append(name)
            if cls == 'note':
                domain[key].append({'case': name, 'notes': v['notes'][:3]})
            if cls == 'unexplained':
                report['unexplained'].append({'case': name, 'pair': key, 'problems': v['problems'][:3]})
        if problems and name in fixed_names:
            report['fixture_problems'].append({'case': name, 'problems': problems})
        if disagree:
            report['pair_disagreements'].append({'case': name, 'detail': disagree})
        done['n'] += 1
        if done['n'] % 100 == 0 or done['n'] == len(cases):
            bad = sum(s['unexplained'] for s in stats.values())
            print('%d/%d cases, %d unexplained, %.0f s' % (done['n'], len(cases), bad, time.monotonic() - t0), flush=True)
    with ThreadPoolExecutor(args.jobs) as ex:
        futures = [ex.submit(run_one, case, exes, args.keep, T, oracle) for case in parallel]
        for fut in futures:
            consume(fut.result())
    for case in serial:                                  # real stalled children: one at a time, no harness overload
        consume(run_one(case, exes, args.keep, T, oracle))
    policy_report = {'seed': args.seed, 'note': 'explained deviations only; unexplained failures and excluded-domain observations are listed separately',
                     'pairs': {}}
    for key, cnt in stats.items():
        report['pairs'][key] = dict(cnt)
        policy_report['pairs'][key] = {'explained': {p: {'cases': len(v), 'first': v[:10]} for p, v in sorted(explained[key].items())},
                                       'excluded_domain_observations': domain[key][:50]}
        print(key, dict(cnt), flush=True)
    report['wall_seconds'] = round(time.monotonic() - t0, 1)
    report['unexplained'] = report['unexplained'][:400]
    report['unexplained_count'] = sum(s['unexplained'] for s in stats.values())
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, 'w', encoding='utf-8') as f:
        json.dump(report, f, indent=1, default=str)
    with open(args.policy_report, 'w', encoding='utf-8') as f:
        json.dump(policy_report, f, indent=1, default=str)
    print('reports written to %s and %s' % (args.out, args.policy_report))
    bad = report['unexplained_count'] + len(report['fixture_problems']) + len(report['pair_disagreements'])
    print('FAILED: %d unexplained, %d fixture problems, %d pair disagreements' % (report['unexplained_count'], len(report['fixture_problems']),
                                                                              len(report['pair_disagreements'])) if bad else 'no unexplained differences')
    return 1 if bad else 0


if __name__ == '__main__':
    raise SystemExit(main())
