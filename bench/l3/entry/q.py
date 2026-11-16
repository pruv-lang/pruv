"""L3-8 entry Q run: seeded random cases (spec section 8) against the pinned-TypeScript oracle and the verified Pruv pairs.

  python3 q.py [--seed 20261118] [--count 2000] [--pairs rs/rust,...] [--reuse-verified] [--families G1,G3]
               [--ops 2,3,4,7] [--jobs N] [--shrink-max 5] [--list]

Generation is deterministic from --seed (xorshift32, spec 8). All cases are generated first and the --families / --ops
filters are applied afterwards, so a case keeps its number. Only the default --count 2000 has the exact family sizes;
other counts scale the 400/300/200/300/300/200/200/100 split proportionally (largest remainder).
Output: work/q/report.json, work/q/fail/<n>.json for each unexplained case. Exit code 1 on any unexplained case."""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import shutil
import struct
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from common import WORK, compare, forbidden_reached, jv_string, load_defaults, merge_case, probe_derivable, probe_expect
import test_impl
from oracle import run_oracle

OPS = ['import_sources', 'find_source', 'list_external', 'read_external', 'account_home', 'codex_account_folder',
       'session_threads', 'excluded_from_bound', 'bound_session_index']
FAMILIES = [('G1', 400), ('G2', 300), ('G3', 200), ('G4', 300), ('G5', 300), ('G6', 200), ('G7', 200), ('G8', 100)]
CLASSES = ['exact', 'P5', 'P6', 'engine-policy', 'excluded-domain', 'unexplained']
QDIR = WORK / 'q'
CHUNK = 250
ENG5 = ['claude', 'codex', 'opencode', 'qwen', 'grok']
CODES = ['not-found', 'codex-unsupported', 'grok-exists', 'grok-load-failed', 'timeout', 'read-failed', 'engine-missing']
DD = '/data'


# ------------------------------------------------------------------------------------------------ PRNG (spec 8)
class Rng:
    def __init__(self, seed):
        self.x = seed & 0xFFFFFFFF or 1

    def word(self):
        x = self.x
        x ^= (x << 13) & 0xFFFFFFFF
        x ^= x >> 17
        x ^= (x << 5) & 0xFFFFFFFF
        self.x = x
        return x

    def below(self, n):
        """Uniform 0..n-1 by rejection sampling (a word is always consumed)."""
        limit = (1 << 32) // n * n
        while True:
            w = self.word()
            if w < limit:
                return w % n

    def between(self, a, b):
        return a + self.below(b - a + 1)

    def chance(self, num, den):
        return self.below(den) < num

    def pick(self, seq):
        return seq[self.below(len(seq))]

    def weighted(self, items):
        total = sum(w for _, w in items)
        k = self.below(total)
        for v, w in items:
            if k < w:
                return v
            k -= w
        return items[-1][0]


def scaled(count):
    total = sum(n for _, n in FAMILIES)
    if count == total:
        return list(FAMILIES)
    raw = [(f, n * count / total) for f, n in FAMILIES]
    out = {f: int(v) for f, v in raw}
    rest = count - sum(out.values())
    for f, _ in sorted(raw, key=lambda fv: -(fv[1] - int(fv[1])))[:rest]:
        out[f] += 1
    return [(f, out[f]) for f, _ in FAMILIES]


# ------------------------------------------------------------------------------------------------ value helpers
def ustr(units):
    return bytes(b for u in units for b in struct.pack('<H', u)).decode('utf-16-le', 'surrogatepass')


def units_of(s):
    raw = s.encode('utf-16-le', 'surrogatepass')
    return [int.from_bytes(raw[i:i + 2], 'little') for i in range(0, len(raw), 2)]


def jvu(units):
    return jv_string(ustr(units))


UNDEF = {'undef': 1}
ABSENT = {'absent': 1}
SPACES = [0x20, 0x09, 0x0A, 0x0D, 0xA0, 0x1680, 0x2003, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000, 0xFEFF, 0x85, 0x200B]


def rstring(r, lo=0, hi=12, rich=True):
    n = r.between(lo, hi)
    out = []
    for _ in range(n):
        k = r.below(12) if rich else r.below(6)
        if k < 6:
            out.append(r.pick([0x61, 0x62, 0x78, 0x2F, 0x3A, 0x2D, 0x5F, 0x30]))
        elif k == 6:
            out.append(r.between(0xAC00, 0xAC40))
        elif k == 7:
            out.append(0x301)
        elif k == 8:
            out.extend([0xD83D, r.between(0xDE00, 0xDE4F)])
        elif k == 9:
            out.append(r.pick([0xD800, 0xDBFF, 0xDC00, 0xDFFF]))
        elif k == 10:
            out.append(r.pick(SPACES))
        else:
            out.append(r.between(0x80, 0x7FF))
    return jvu(out)


def rtext(r, lo=0, hi=12):
    """Well-formed string only (usable as an object key)."""
    n = r.between(lo, hi)
    return ''.join(r.pick(['a', 'b', 'x', '/', ':', '-', '_', '0', 'é', '中', '\U0001F600', ' ']) for _ in range(n))


def ident(r):
    return 'acc_' + ''.join(r.pick('0123456789abcdef') for _ in range(16))


BAD_IDS = ['acc_0123456789ABCDEF', 'acc_0123456789abcde', 'acc_0123456789abcdef0', 'acc_0123456789abcdef\n', 'acc_0123456789/abcdef',
           '../acc_0123456789abcdef', '', ' acc_0123456789abcdef', 'ACC_0123456789abcdef', 'acc_٣123456789abcdef']


def gen_err(r, ref):
    cls = r.weighted([('Error', 4), ('TypeError', 1), ('EngineImportError', 4), ('plain', 3), ('primitive', 3)])
    if cls == 'primitive':
        return {'class': 'primitive', 'ref': ref, 'value': r.pick([None, UNDEF, 'oops', 42, False, True, 0, '', jvu([0xD800])])}
    out = {'class': cls, 'ref': ref}
    if cls == 'EngineImportError':
        out['code'] = r.pick(CODES)
        if r.chance(1, 4):
            out['rename'] = r.pick(['', 'SomethingElse', 'AbortError'])
    elif r.chance(1, 3):
        out['name'] = r.pick(['AbortError', 'AbortError', 'aborterror', 'AbortError ', 'EngineImportError', 'TimeoutError', 'Error', ''])
    if cls != 'EngineImportError' or r.chance(1, 2):
        m = r.below(5)
        if m == 1:
            out['message'] = ''
        elif m == 2:
            out['message'] = r.pick(['   ', ' \t\n'])
        elif m == 3:
            out['message'] = rstring(r, 1, 10)
        elif m == 4:
            out['message'] = jvu([0x64, 0xE9, 0xD800, 0x20, 0xD83D, 0xDE00])
    return out


# ------------------------------------------------------------------------------------------------ G1 import / accounts
ENV_KEYS = ['CLAUDE_CONFIG_DIR', 'GROK_HOME', 'CODEX_HOME', 'HOME', 'QWEN_HOME', 'QWEN_RUNTIME_DIR', 'XDG_DATA_HOME']


def env_value(r):
    k = r.below(12)
    if k == 0:
        return ''
    if k == 1:
        return r.pick(['   ', ' \t\n ', ' \u0085', '​', ' ', '　 '])
    if k == 2:
        return ' \t/pad/abs\n '
    if k == 3:
        return r.pick(['rel/path', './rel', '../up/rel', 'x'])
    if k == 4:
        return '/'
    if k == 5:
        return r.pick(['/a/./b/../c', '/a//b', '/a/b/', '/..', '/a/../../b'])
    if k == 6:
        return r.pick(['/höme/☃', '/中文/\U0001F600', '/é'])
    if k == 7:
        return r.pick(['﻿/bom/home', ' /em/home ', '\u0085/nel', '/zwsp​', '​/lead'])
    return '/' + '/'.join(rtext(r, 1, 5).replace('/', '_') for _ in range(r.between(1, 3)))


def gen_accounts(r, ambient_given):
    out = {}
    for eng in ('claude', 'codex'):
        m = r.below(10)
        if m == 0:
            continue
        if m == 1:
            out[eng] = {'throw': gen_err(r, 'cb_' + eng)}
            continue
        n = r.weighted([(0, 3), (1, 3), (2, 3), (3, 2), (r.between(4, 12), 2)])
        rows, seen = [], []
        for _ in range(n):
            k = r.below(10)
            if k < 6:
                i = ident(r)
            elif k < 8 and seen:
                i = r.pick(seen)
            else:
                i = r.pick(BAD_IDS)
            seen.append(i)
            name = r.pick(['n', '', 'Name', jvu([0x4E2D, 0xD800]), '  pad  ', rstring(r, 0, 6)]) if r.chance(1, 2) else 'n'
            rows.append({'id': i, 'name': name})
        out[eng] = {'ok': rows}
    return out


def gen_g1(r):
    if r.chance(1, 5):
        eng = r.pick(['claude', 'codex', 'claude', 'codex', 'grok', ''])
        i = r.pick([ident(r), ident(r)] + BAD_IDS)
        return {'op': 'account_home', 'args': {'engine': eng, 'id': i}}
    env = {k: env_value(r) for k in ENV_KEYS if r.chance(1, 2)}
    script = {}
    amb = None
    k = r.below(5)
    if k == 1:
        amb = env_value(r)
        script['ambient'] = [{'ok': amb}]
    elif k == 2:
        script['ambient'] = [{'ok': UNDEF}]
    elif k == 3:
        script['ambient'] = [{'ok': '/amb'}]
        amb = '/amb'
    acc = gen_accounts(r, amb is not None)
    if acc:
        script['accounts'] = acc
    if r.chance(1, 25):
        script['homedir'] = [{'throw': gen_err(r, 'hd')}]
    elif r.chance(1, 8):
        script['homedir'] = [{'ok': env_value(r) or '/hm'}]
    if amb and acc.get('codex') and 'ok' in acc['codex'] and acc['codex']['ok'] and r.chance(1, 2):
        # alias some codex account folder to the ambient home through realpath
        chosen = r.pick(acc['codex']['ok'])
        script['realpath'] = {f'{DD}/engine-homes/codex/{chosen["id"]}': [{'ok': '/real/x'}], amb: [{'ok': '/real/x'}]}
    return {'op': 'import_sources', 'args': {'env': env}, 'script': script}


# ------------------------------------------------------------------------------------------------ G2 codex account folder
def gen_g2(r):
    extra = r.weighted([(0, 3), (1, 3), (2, 3), (3, 2), (4, 1), (5, 1)])
    rel = r.chance(1, 8)
    data = ('rel/' if rel else '/') + '/'.join(['d%d' % i for i in range(extra)] + ['data'])
    i = ident(r) if not r.chance(1, 10) else r.pick(BAD_IDS[:3])
    target = f'{data}/engine-homes/codex/{i}'
    home = target if not r.chance(1, 12) else target + '/x'
    cwd = '/cwdx'
    abs_target = (cwd + '/' + target) if rel else target
    chain = [abs_target]
    p = abs_target
    while p != '/':
        p = p.rsplit('/', 1)[0] or '/'
        chain.append(p)
    amb_kind = r.below(6)
    amb = {0: None, 1: '/amb', 2: abs_target, 3: '/real/x', 4: '/home/test/.codex', 5: chain[r.below(len(chain))]}[amb_kind]
    real_pool = ['/real/x', '/real/y', '/real/x/' + i, '/real/z']
    realpath, stat = {}, {}
    paths = list(dict.fromkeys(chain + [x for x in (amb, '/home/test/.codex') if x]))
    for path in paths:
        if r.chance(1, 3):
            q = []
            for _ in range(r.weighted([(1, 5), (2, 2), (3, 1)])):
                m = r.below(8)
                q.append({'ok': r.pick(real_pool)} if m < 4 else {'errno': r.pick(['ENOENT', 'EACCES', 'ELOOP', 'EIO', 'EACCES']),
                                                                  **({'ref': 'rpe'} if r.chance(1, 5) else {})})
            realpath[path] = q
    if r.chance(1, 2) and 'ok' not in json.dumps(realpath.get(abs_target, [])):
        # make a level of the chain exist (missing leaf components)
        realpath[chain[r.between(1, len(chain) - 1)]] = [{'ok': r.pick(real_pool)}]
    devs = [('1', '2'), ('1', '2'), ('3', '4'), ('5', '6'), ('9007199254740993', '18446744073709551615')]
    for path in paths:
        if r.chance(1, 2):
            q = []
            for _ in range(r.weighted([(1, 4), (2, 3), (3, 1)])):
                if r.chance(1, 5):
                    q.append({'errno': r.pick(['ENOENT', 'EACCES', 'EIO'])})
                else:
                    d, n = r.pick(devs)
                    q.append({'ok': {'dev': d, 'ino': n}})
            stat[path] = q
    script = {}
    if amb is not None:
        script['ambient'] = [{'ok': amb}]
    if realpath:
        script['realpath'] = realpath
    if stat:
        script['stat'] = stat
    if rel:
        script['cwd'] = [{'ok': cwd}]
    return {'op': 'codex_account_folder', 'args': {'dataDir': data, 'account': {'id': i, 'home': home}}, 'script': script}


# ------------------------------------------------------------------------------------------------ G3 find / route
ID_POOL = ['claude:home', 'codex:home', 'grok:home', 'a:b', 'a:b:c', '', ' claude:home ', 'Claude:Home', 'x', 'acc', ':', 'é',
           'claude:acc_0123456789abcdef', 'qwen:home', 'opencode:home']


def src_id(r):
    k = r.below(10)
    if k < 6:
        return r.pick(ID_POOL)
    if k == 6:
        return jvu([0xD800])
    if k == 7:
        return jvu([0xDC00, 0xD800])
    if k == 8:
        return '�'
    return rtext(r, 0, 8)


def gen_find(r):
    n = r.weighted([(0, 2), (1, 3), (r.between(2, 6), 6), (r.between(7, 32), 3)])
    ids = [src_id(r) for _ in range(n)]
    if n and r.chance(1, 3):
        ids[r.below(n)] = ids[r.below(n)]
    if ids and r.chance(2, 3):
        q = r.pick(ids)
        v = r.below(6)
        if v == 1 and isinstance(q, str):
            q = q.upper()
        elif v == 2 and isinstance(q, str):
            q = ' ' + q + ' '
        elif v == 3:
            q = ''
    else:
        q = src_id(r)
    sources = []
    for s in ids:
        e = {'source': s}
        if r.chance(1, 4):
            e['home'] = rtext(r, 0, 5)
        if r.chance(1, 6):
            e['engine'] = r.pick(ENG5)
        sources.append(e)
    return {'op': 'find_source', 'args': {'sources': sources, 'query': q}}


def gen_route(r, case_no):
    engine = r.weighted([(e, 3) for e in ENG5] + [('gemini', 1), ('', 1), ('Claude', 1), ('claude ', 1), ('CODEX', 1)])
    src = {'ref': 's1', 'engine': engine}
    if r.chance(1, 3):
        src['source'] = r.pick(ID_POOL) if r.chance(1, 2) else '%s:home' % engine
    if r.chance(1, 4):
        src['home'] = rtext(r, 0, 6)
    args = {'source': src, 'folder': rstring(r, 0, 8) if r.chance(1, 3) else '/f', 'options': {'ref': 'o%d' % case_no}}
    if r.chance(1, 2):
        args['thread'] = rstring(r, 0, 8) if r.chance(1, 3) else 't'
        return {'op': 'read_external', 'args': args, 'script': {'engine': {'record': 'rec%d' % case_no}}}
    rows = [{'ref': 'r%d_%d' % (case_no, i), 'title': 't%d' % i} for i in range(r.between(0, 4))]
    return {'op': 'list_external', 'args': args, 'script': {'engine': {'rows': rows}}}


def gen_g3(r, case_no):
    return gen_find(r) if r.chance(1, 2) else gen_route(r, case_no)


# ------------------------------------------------------------------------------------------------ G4 titles
def title_units(r):
    kind = r.weighted([('len0', 1), ('len1', 2), ('edge', 5), ('mid', 3), ('tail', 1)])
    if kind == 'len0':
        return []
    if kind == 'len1':
        return [r.pick([0x61, 0xD800, 0xDC00, 0x20, 0x301, 0xFFFF])]
    if kind == 'edge':
        n = r.pick([197, 198, 199, 199, 200, 200, 201, 202])
    elif kind == 'mid':
        n = r.between(2, 400)
    else:
        n = r.between(401, 16384)
    out = []
    while len(out) < n:
        k = r.below(14)
        if k < 7:
            out.append(r.between(0x61, 0x7A))
        elif k == 7:
            out.append(r.between(0x4E00, 0x4E40))
        elif k == 8:
            out.append(0x301)
        elif k == 9:
            out.extend([0xD83D, r.between(0xDE00, 0xDE4F)])
        elif k == 10:
            out.append(r.pick([0xD800, 0xDBFF, 0xDC00, 0xDFFF]))
        elif k == 11:
            out.append(r.pick(SPACES))
        elif k == 12:
            out.append(r.between(0x80, 0x7FF))
        else:
            out.append(0x78)
    out = out[:n]
    if n >= 200 and r.chance(1, 2):  # bias to the 199/200 boundary
        k = r.below(5)
        if k == 0:
            out[198], out[199] = 0xD83D, 0xDE00
        elif k == 1:
            out[199] = 0xD83D
        elif k == 2:
            out[199], out[200 if n > 200 else 199] = 0xDE00, 0x61
        elif k == 3:
            out[198:200] = [0x301, 0x301]
        else:
            out[199] = 0xD800
    return out


def gen_g4(r, case_no):
    kind = r.below(3)
    n = r.weighted([(0, 1), (1, 3), (r.between(2, 8), 4), (r.between(9, 64), 2)])
    rows = []
    for i in range(n):
        row = {'ref': 'r%d_%d' % (case_no, i), 'title': jvu(title_units(r))}
        if r.chance(1, 3):
            row['id'] = 'i%d' % i
        if r.chance(1, 6):
            row['n'] = r.pick([1, 0, {'num': '3ff8000000000000'}, {'num': '8000000000000000'}])
        if r.chance(1, 8):
            row['o'] = {'ref': 'ox%d' % i}
        if r.chance(1, 8):
            row['a'] = {'arr': [r.pick(['p', None, UNDEF, 3])]}
        if r.chance(1, 10):
            row['u'] = UNDEF
        rows.append(row)
    engine = r.pick(ENG5)
    return {'op': 'list_external', 'args': {'source': {'ref': 's1', 'engine': engine}, 'folder': '/f', 'options': {'ref': 'o1'}},
            'script': {'engine': {'rows': rows}}}


# ------------------------------------------------------------------------------------------------ G5 errors
def gen_g5(r, case_no):
    op = r.pick(['list_external', 'read_external'])
    e = gen_err(r, 'e%d' % case_no)
    mode = 'reject' if r.chance(2, 3) else 'sync_throw'
    args = {'source': {'ref': 's1', 'engine': r.pick(ENG5)}, 'folder': '/f', 'options': {'ref': 'o1'}}
    if op == 'read_external':
        args['thread'] = 't'
    return {'op': op, 'args': args, 'script': {'engine': {mode: e}}}


# ------------------------------------------------------------------------------------------------ G6 threads
def thread_value(r, domain=True):
    k = r.weighted([('s', 8), ('e', 1), ('n', 1), ('u', 1), ('i', 1), ('sur', 1)])
    if k == 's':
        return r.pick(['t', 't1', 't2', 'f', 'x:y']) if r.chance(2, 3) else rstring(r, 1, 8)
    if k == 'e':
        return ''
    if k == 'n':
        return None
    if k == 'u':
        return UNDEF
    if k == 'i':
        return r.pick([0, 1, 7, -3, 9007199254740991])
    return jvu([0x74, 0xD800, 0xDC00, 0xDC00])


def slot_key(r):
    k = r.weighted([('c', 6), ('nc', 2), ('mc', 2), ('lead', 1), ('empty', 1), ('int', 2)])
    if k == 'c':
        return r.pick(ENG5 + ['mystery']) + ':' + rtext(r, 0, 4).replace(':', '')
    if k == 'nc':
        return r.pick(ENG5 + ['x'])
    if k == 'mc':
        return 'a:' + rtext(r, 0, 3) + ':b:c'
    if k == 'lead':
        return ':' + rtext(r, 0, 3)
    if k == 'empty':
        return ''
    return r.pick(['0', '1', '2', '10', '01', '-1', '4294967295', '4294967294', '7'])


def gen_session(r, sid, shape=None):
    s = {'id': sid}
    if r.chance(1, 3):
        s['engine'] = r.pick(ENG5 + ['mystery', ''])
    if r.chance(1, 2):
        et = {'engine': r.pick(ENG5 + ['mystery', '']) if r.chance(9, 10) else r.pick([None, UNDEF]), 'thread': thread_value(r)}
        if r.chance(1, 2):
            f = r.below(4)
            if f == 0:
                et['forks'] = None
            elif f == 1:
                et['forks'] = UNDEF
            else:
                et['forks'] = {'arr': [thread_value(r) for _ in range(r.between(0, 5 if f == 2 else 12))]}
        s['externalThread'] = et
    elif r.chance(1, 12):
        s['externalThread'] = r.pick([None, UNDEF])
    if r.chance(1, 4):
        t = r.below(4)
        if t == 0:
            s['terminal'] = None
        else:
            s['terminal'] = {'cliSession': r.pick(['cli', '', None, UNDEF, 0, 1, 'c2']), 'started': r.chance(1, 2)}
            if r.chance(1, 3):
                s['terminal']['engine'] = r.pick(ENG5 + [''])
    if r.chance(1, 3):
        n = r.weighted([(0, 1), (1, 3), (2, 3), (r.between(3, 12), 2)])
        pairs = []
        for _ in range(n):
            b = r.weighted([('t', 8), ('null', 1), ('undef', 1), ('absent', 1), ('tnull', 1), ('empty', 1), ('other', 1)])
            val = {'t': {'threadId': thread_value(r)}, 'null': None, 'undef': UNDEF, 'absent': ABSENT, 'tnull': {'threadId': None},
                   'empty': {'threadId': ''}, 'other': {'other': 1}}[b]
            if b == 't' and val['threadId'] in (None, UNDEF) and r.chance(1, 2):
                val = {'threadId': 'z'}
            pairs.append([slot_key(r), val])
        s['engineThreads'] = {'obj': pairs}
    return s


def gen_g6(r):
    return {'op': 'session_threads', 'args': {'session': gen_session(r, 's')}}


# ------------------------------------------------------------------------------------------------ G7 bound index
TEMP_VALUES = [True, True, False, None, UNDEF, 1, 0, 'true', '', 'yes']


def gen_g7(r, case_no):
    excluded_ok = r.chance(3, 100)
    if r.chance(1, 4):
        s = gen_session(r, 's')
        if r.chance(3, 4):
            s['temporary'] = r.pick(TEMP_VALUES)
        script = {}
        q = []
        for _ in range(r.between(0, 3)):
            q.append({'ok': r.chance(1, 2)} if not r.chance(1, 8) else {'throw': gen_err(r, 'tx%d' % len(q))})
        if q:
            script['isTemporary'] = q
        case = {'op': 'excluded_from_bound', 'args': {'session': s}}
        if script:
            case['script'] = script
        return case
    n = r.weighted([(0, 1), (1, 2), (r.between(2, 8), 5), (r.between(9, 32), 2)])
    sessions = []
    pairs_pool = [(r.pick(ENG5 + ['mystery']), r.pick(['t', 't1', 'a:b', 'b', 'x'])) for _ in range(5)]
    pairs_pool += [('a', 'b:c'), ('a:b', 'c')]
    for i in range(n):
        sid = r.pick(['s%d' % i, 's%d' % i, 's%d' % i, '', 's0']) if r.chance(1, 4) else 's%d' % i
        s = {'id': sid}
        eng, th = r.pick(pairs_pool)
        if r.chance(3, 4):
            s['externalThread'] = {'engine': eng, 'thread': th}
            if r.chance(1, 4):
                s['externalThread']['forks'] = {'arr': [r.pick(pairs_pool)[1] for _ in range(r.between(0, 3))]}
        if r.chance(1, 5):
            s['engineThreads'] = {'obj': [['%s:%s' % (r.pick(ENG5), r.pick(['a', 'b'])), {'threadId': r.pick(['t', 't1', 'x'])}] for _ in range(r.between(0, 3))]}
        if r.chance(1, 5):
            s['terminal'] = {'cliSession': r.pick(['cli', 't', '', None]), 'started': True}
        if r.chance(2, 3):
            s['temporary'] = r.pick(TEMP_VALUES)
        sessions.append(s)
    script = {}
    q = []
    for _ in range(r.weighted([(0, 3), (1, 3), (2, 2), (r.between(3, 12), 1)])):
        q.append({'ok': r.chance(1, 3)} if not r.chance(1, 12) else {'throw': gen_err(r, 'tx%d' % len(q))})
    if q:
        script['isTemporary'] = q
    case = {'op': 'bound_session_index', 'args': {'sessions': sessions}}
    if script:
        case['script'] = script
    if excluded_ok and sessions:
        s = r.pick(sessions)
        s['externalThread'] = {'engine': {'ref': 'badeng'}, 'thread': 'x'}
        s.pop('temporary', None)
        case['tag'] = 'excluded_ok'
    return case


# ------------------------------------------------------------------------------------------------ G8 packets
CORRUPT = ['version', 'length', 'tag', 'unit', 'op_id', 'seq', 'duplicate', 'reentry', 'oversize', 'exact_limit']


def long_thread(r):
    n = r.weighted([(r.between(1, 20), 4), (r.between(200, 3000), 3), (r.between(3000, 20000), 1)])
    return jvu([r.pick([0x61, 0x62, 0xD83D, 0xDE00, 0xD800, 0xDC00, 0x301, 0x4E2D, 0x20]) for _ in range(n)])


def gen_g8(r, case_no):
    k = r.below(6)
    if k == 0:
        case = {'op': 'session_threads', 'args': {'session': {'id': 's', 'externalThread': {'engine': 'claude', 'thread': long_thread(r),
                                                                                         'forks': {'arr': [long_thread(r) for _ in range(r.between(0, 4))]}}}}}
    elif k == 1:
        case = gen_g7(r, case_no)
        case.pop('tag', None)
    elif k == 2:
        case = gen_g4(r, case_no)
    elif k == 3:
        case = gen_g1(r)
    elif k == 4:
        case = gen_g5(r, case_no)
    else:
        case = gen_g6(r)
    if r.chance(2, 5) or case['op'] in ('import_sources',) and r.chance(1, 2):
        case['probe'] = {'at': r.below(4), 'corrupt': r.weighted([(c, 1) if c not in ('oversize', 'exact_limit') else (c, 1) for c in CORRUPT])}
    return case


# ------------------------------------------------------------------------------------------------ generation
def generate(seed, count):
    r = Rng(seed)
    multiset = [f for f, n in scaled(count) for _ in range(n)]
    for i in range(len(multiset) - 1, 0, -1):
        j = r.below(i + 1)
        multiset[i], multiset[j] = multiset[j], multiset[i]
    cases = []
    for no, fam in enumerate(multiset, 1):
        g = {'G1': lambda: gen_g1(r), 'G2': lambda: gen_g2(r), 'G3': lambda: gen_g3(r, no), 'G4': lambda: gen_g4(r, no),
             'G5': lambda: gen_g5(r, no), 'G6': lambda: gen_g6(r), 'G7': lambda: gen_g7(r, no), 'G8': lambda: gen_g8(r, no)}[fam]()
        g['id'] = 'Q%04d' % no
        g['family'] = fam
        g['n'] = no
        cases.append(g)
    return cases


# ------------------------------------------------------------------------------------------------ running
def full(case, defaults):
    """The case as sent to the oracle / harness: defaults merged, bookkeeping keys removed (the probe stays for the harness)."""
    c = {k: v for k, v in case.items() if k not in ('family', 'n', 'tag', 'oracle_id')}
    merged = merge_case(defaults, c)
    merged.pop('probe', None)
    return merged


def run_safe(fn, items, label):
    """fn(items) -> results; on failure bisect so one crashing case cannot hide the rest. A lone failure -> ('crash', text)."""
    try:
        return fn(items)
    except Exception as exc:  # noqa: BLE001
        if len(items) == 1:
            return [{'__crash__': f'{label}: {str(exc)[-400:]}'}]
        mid = len(items) // 2
        return run_safe(fn, items[:mid], label) + run_safe(fn, items[mid:], label)


def chunked(items, size):
    return [items[i:i + size] for i in range(0, len(items), size)]


def probe_clamp(case, base):
    """Adjust the probe position to the oracle trace of the base case (deterministic). Returns the probe dict."""
    p = dict(case['probe'])
    n = len(base.get('trace', [])) if isinstance(base, dict) else 0
    if p['corrupt'] == 'duplicate' and (n < 2 or case['op'] != 'import_sources'):
        p['corrupt'] = 'version'  # a duplicated completion is reachable only through op 1 (spec 9, P08)
    p['at'] = min(p['at'], max(n - 1, 0)) if p['corrupt'] != 'duplicate' else max(1, min(p['at'], n - 1))
    if case['op'] != 'import_sources':
        p['at'] = 0  # the other ops send their requests in one await (or none): only the begin packet is a step boundary
    # Keep the position derivable (account_at awaits are untraced; the codex account phase is not derivable).
    trace = base.get('trace', []) if isinstance(base, dict) else []
    while p['at'] > 0 and not probe_derivable(p['at'], trace, case.get('script')):
        p['at'] -= 1
    if p['corrupt'] == 'duplicate' and p['at'] < 1:
        p['corrupt'] = 'version'
    return p


def evaluate(cases, exes, jobs, defaults):
    """cases -> {id: {'oracle': res, 'probe': probe|None, 'pairs': {pair: res}}} with the oracle and each pair run in chunks."""
    sendable = [full(c, defaults) for c in cases]
    jobs = max(1, jobs)
    with ThreadPoolExecutor(jobs) as pool:
        ofut = [pool.submit(run_safe, run_oracle, ch, 'oracle') for ch in chunked(sendable, CHUNK)]
        oracle = [x for f in ofut for x in f.result()]
    oracle_by = {c['id']: o for c, o in zip(cases, oracle)}
    # probes: derive position from the oracle trace, then send the probe to the harness
    run_cases = []
    probes = {}
    for c, s in zip(cases, sendable):
        s = dict(s)
        if 'probe' in c:
            p = probe_clamp(c, oracle_by[c['id']])
            probes[c['id']] = p
            s['probe'] = p
        run_cases.append(s)
    out = {c['id']: {'oracle': oracle_by[c['id']], 'probe': probes.get(c['id']), 'pairs': {}} for c in cases}
    for key, exe in exes.items():
        with ThreadPoolExecutor(jobs) as pool:
            futs = [pool.submit(run_safe, lambda ch, e=exe: test_impl.run_runner(e, ch), ch, key) for ch in chunked(run_cases, CHUNK)]
            res = [x for f in futs for x in f.result()]
        for c, a in zip(cases, res):
            out[c['id']]['pairs'][key] = a
    return out


def refused(o):
    return not isinstance(o, dict) or '__crash__' in o or 'error' in o or 'terminal' not in o


def judge(case, rec, pair):
    """-> (class, field) of one pair's result for one case."""
    a = rec['pairs'][pair]
    o = rec['oracle']
    if not isinstance(a, dict) or '__crash__' in a:
        return 'unexplained', 'crash'
    if 'error' in a or 'terminal' not in a:
        return 'unexplained', 'harness-error'
    if a.get('id') != case['id']:
        return 'unexplained', 'id'
    if refused(o):
        return 'unexplained', 'oracle-refused'
    if 'InternalInvariant' in json.dumps(a.get('value')) and 'InternalInvariant' not in json.dumps(o.get('value')):
        return 'unexplained', 'InternalInvariant'
    probe = rec['probe']
    if probe:
        c = probe['corrupt']
        if c == 'exact_limit':
            return compare(o, a)
        exp = probe_expect(probe, o, case.get('script'))
        cls, field = compare(exp, a, probe_too_large=(c == 'oversize'))
        if cls == 'exact' and c == 'duplicate' and a.get('driver') != 'InvalidPacket':
            return 'unexplained', 'driver'
        return cls, field
    return compare(o, a, excluded=forbidden_reached(case))


def classify(cases, recs, pairs):
    """-> per case {'classes': {pair: (cls, field)}, 'cross': (cls, field)|None, 'worst': cls}."""
    rows = {}
    for c in cases:
        rec = recs[c['id']]
        classes = {p: judge(c, rec, p) for p in pairs}
        cross = None
        base = rec['pairs'][pairs[0]]
        for p in pairs[1:]:
            a = rec['pairs'][p]
            if isinstance(base, dict) and isinstance(a, dict) and 'terminal' in base and 'terminal' in a:
                cl, fld = compare(base, a)
                if cl != 'exact':
                    cross = ('unexplained', f'cross-pair {pairs[0]} vs {p}: {fld}')
        worst = 'exact'
        for cls, _ in list(classes.values()) + ([cross] if cross else []):
            if CLASSES.index(cls) > CLASSES.index(worst):
                worst = cls
        rows[c['id']] = {'classes': classes, 'cross': cross, 'worst': worst}
    return rows


# ------------------------------------------------------------------------------------------------ shrinking
def shrink_variants(node, limit=60):
    """Smaller versions of a JSON node (rows, sessions, accounts, suffix components, keys, string units)."""
    if isinstance(node, list):
        for i in range(len(node)):
            yield node[:i] + node[i + 1:]
        for i, x in enumerate(node):
            for y in shrink_variants(x, limit):
                yield node[:i] + [y] + node[i + 1:]
    elif isinstance(node, dict):
        for k in list(node):
            if k != 'u16':
                yield {kk: v for kk, v in node.items() if kk != k}
        for k, v in node.items():
            for y in shrink_variants(v, limit):
                yield {**node, k: y}
    elif isinstance(node, str) and len(node) > 0:
        u = units_of(node)
        n = len(u)
        if n > 32:
            yield jv_to_str(u[:n // 2])
            yield jv_to_str(u[n // 2:])
        for i in range(min(n, 40)):
            yield jv_to_str(u[:i] + u[i + 1:])


def jv_to_str(units):
    v = jvu(units)
    return v if isinstance(v, str) else v  # lone surrogates become a u16 object; callers accept both


def shrink_case(case, pair, exes, defaults, rounds=30):
    """Greedy: accept the first smaller candidate that still fails unexplained on `pair` (a real result, no harness error)."""
    cur = case
    for _ in range(rounds):
        cands = []
        for part in ('args', 'script'):
            if part not in cur:
                continue
            for v in shrink_variants(cur[part]):
                cands.append({**cur, part: v})
                if len(cands) >= 400:
                    break
        if not cands:
            break
        for i, c in enumerate(cands):
            c['id'] = 'S%04d' % i
        recs = evaluate(cands, {pair: exes[pair]}, 1, defaults)
        picked = None
        for c in cands:
            rec = recs[c['id']]
            a = rec['pairs'][pair]
            if refused(rec['oracle']) or not isinstance(a, dict) or 'terminal' not in a or 'error' in a:
                continue
            if judge(c, rec, pair)[0] == 'unexplained':
                picked = c
                break
        if not picked:
            break
        picked['id'] = case['id']
        cur = picked
    return cur


# ------------------------------------------------------------------------------------------------ main
def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--seed', type=int, default=20261118)
    ap.add_argument('--count', type=int, default=2000)
    ap.add_argument('--pairs', help='comma list of py/rust,py/llvm,rs/rust,rs/llvm (default all four)')
    ap.add_argument('--reuse-verified', action='store_true')
    ap.add_argument('--families', help='comma list, e.g. G1,G3 (applied after generation)')
    ap.add_argument('--ops', help='comma list of op names or numbers 1..9 (applied after generation)')
    ap.add_argument('--jobs', type=int, default=4)
    ap.add_argument('--shrink-max', type=int, default=5)
    ap.add_argument('--list', action='store_true', help='print the generated cases (after filtering) and exit')
    args = ap.parse_args(argv)

    all_cases = generate(args.seed, args.count)
    cases = all_cases
    if args.families:
        want = {f.strip().upper() for f in args.families.split(',')}
        cases = [c for c in cases if c['family'] in want]
    if args.ops:
        want = set()
        for o in args.ops.split(','):
            o = o.strip()
            want.add(OPS[int(o) - 1] if o.isdigit() else o)
        cases = [c for c in cases if c['op'] in want]
    if args.list:
        for c in cases:
            print(json.dumps(c, ensure_ascii=False))
        return 0
    if not cases:
        print('no case selected')
        return 2

    defaults = load_defaults()
    pairs_t = test_impl.parse_pairs(args.pairs)
    g = test_impl.configure()
    host = test_impl.build_host()
    exes, failed_build = {}, False
    for compiler, backend in pairs_t:
        key = f'{compiler}_{backend}'
        try:
            exes[key] = test_impl.build_pair(g, compiler, backend, host, args.reuse_verified)
        except Exception as exc:  # noqa: BLE001
            failed_build = True
            print(f'{key}: BUILD ERROR {exc}', file=sys.stderr, flush=True)
    if not exes:
        return 1
    pairs = list(exes)

    recs = evaluate(cases, exes, args.jobs, defaults)
    rows = classify(cases, recs, pairs)

    counts = {c: 0 for c in CLASSES}
    per_family = {}
    per_pair = {p: {c: 0 for c in CLASSES} for p in pairs}
    for c in cases:
        row = rows[c['id']]
        counts[row['worst']] += 1
        pf = per_family.setdefault(c['family'], {k: 0 for k in CLASSES})
        pf[row['worst']] += 1
        for p, (cls, _) in row['classes'].items():
            per_pair[p][cls] += 1

    fail_dir = QDIR / 'fail'
    if fail_dir.exists():
        shutil.rmtree(fail_dir)
    fail_dir.mkdir(parents=True, exist_ok=True)
    failing = [c for c in cases if rows[c['id']]['worst'] == 'unexplained']
    shrunk_n = 0
    for c in failing:
        row = rows[c['id']]
        pair = next((p for p, (cls, _) in row['classes'].items() if cls == 'unexplained'), pairs[0])
        first = row['classes'][pair][1] if row['classes'][pair][0] == 'unexplained' else (row['cross'] or ('', ''))[1]
        shrunk = None
        if shrunk_n < args.shrink_max and row['classes'][pair][0] == 'unexplained' and row['classes'][pair][1] not in ('crash', 'harness-error', 'oracle-refused'):
            shrunk_n += 1
            try:
                shrunk = shrink_case(c, pair, exes, defaults)
                shrunk = {k: v for k, v in shrunk.items() if k in ('op', 'args', 'script')}
            except Exception as exc:  # noqa: BLE001
                shrunk = {'error': str(exc)[-300:]}
        rec = recs[c['id']]
        data = {'seed': args.seed, 'count': args.count, 'case': c['n'], 'id': c['id'], 'family': c['family'],
                'input': {k: v for k, v in c.items() if k in ('op', 'args', 'script', 'probe', 'tag')},
                'probe_used': rec['probe'], 'oracle': rec['oracle'], 'pruv': rec['pairs'], 'pair': pair, 'first_differing_field': first,
                'classes': {p: list(v) for p, v in row['classes'].items()}, 'cross': row['cross'], 'shrunk': shrunk}
        (fail_dir / f'{c["n"]}.json').write_text(json.dumps(data, ensure_ascii=False, indent=1) + '\n', encoding='utf-8')

    report = {'seed': args.seed, 'count': args.count, 'generated': len(all_cases), 'selected': len(cases), 'pairs': pairs,
              'families': args.families, 'ops': args.ops, 'classes': counts, 'per_family': per_family, 'per_pair': per_pair,
              'unexplained_cases': [c['n'] for c in failing], 'build_errors': failed_build}
    QDIR.mkdir(parents=True, exist_ok=True)
    (QDIR / 'report.json').write_text(json.dumps(report, indent=1) + '\n', encoding='utf-8')

    print(f'seed {args.seed}: {len(cases)} of {len(all_cases)} generated cases, pairs {", ".join(pairs)}')
    print('family  ' + ''.join(f'{k:>16}' for k in CLASSES))
    for fam in sorted(per_family):
        print(f'{fam:<8}' + ''.join(f'{per_family[fam][k]:>16}' for k in CLASSES))
    print(f'{"total":<8}' + ''.join(f'{counts[k]:>16}' for k in CLASSES))
    if failing:
        print(f'{len(failing)} unexplained; first: ' + ', '.join(f'case {c["n"]} ({c["family"]}/{c["op"]})' for c in failing[:8]))
        print(f'details in {fail_dir}')
    return 1 if (failing or failed_build) else 0


if __name__ == '__main__':
    sys.exit(main())
