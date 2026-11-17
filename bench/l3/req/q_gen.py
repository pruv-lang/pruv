"""L3-9 Q generator: 2,000 random scenarios for the engine-import request handlers (spec section 10).
  python3 q_gen.py [--out work/q/cases.jsonl] [--seed 20261119]
Seed 20261119, xorshift32 with rejection-sampled uniform, a fixed family multiset (Q1 200, Q2 200, Q3 300, Q4 300, Q5 250, Q6 250,
Q7 250, Q8 250) then a Fisher-Yates shuffle. Cases are generated in shuffled order from one random stream, so the output is a pure
function of the seed. Every case is a scenario (SCENARIO.md) plus the keys `family` (Q1..Q8), `tags` and, when set, `tieBreak`
(Q7 only) and `corrupt` (Q8 only). The oracle ignores those extra keys; an implementation harness may read them.
Case ids: Q<family digit>-<ordinal within the family in output order, 4 digits>."""
import sys
sys.dont_write_bytecode = True
import json
from pathlib import Path

from common import HERE
from gen_fixtures import (ACC1, ACC2, ADV, CANCEL, COPY_DEFER, ISO, LIST, OPEN, SETTLE, SRC, abort_err, collect_errs, eie, err, ext,
                          gworld, item, merge, okread, readers, record, sess, slots, turn)

SEED = 20261119
FAMILIES = [('Q1', 200), ('Q2', 200), ('Q3', 300), ('Q4', 300), ('Q5', 250), ('Q6', 250), ('Q7', 250), ('Q8', 250)]
EIE_CODES = ['not-found', 'codex-unsupported', 'grok-exists', 'grok-load-failed', 'timeout', 'read-failed', 'engine-missing', 'zzz', '']
ENGINES = ['claude', 'codex', 'grok', 'opencode', 'qwen']


# ---- random stream ------------------------------------------------------------------------------------------------------
class Rng:
    """xorshift32; uniform below(n) by rejection sampling (always consumes at least one word)."""
    def __init__(self, seed):
        self.s = (seed & 0xFFFFFFFF) or 1

    def next32(self):
        x = self.s
        x ^= (x << 13) & 0xFFFFFFFF
        x ^= x >> 17
        x ^= (x << 5) & 0xFFFFFFFF
        self.s = x
        return x

    def below(self, n):
        limit = (1 << 32) // n * n
        while True:
            x = self.next32()
            if x < limit:
                return x % n

    def pick(self, seq): return seq[self.below(len(seq))]
    def chance(self, num, den=100): return self.below(den) < num
    def between(self, lo, hi): return lo + self.below(hi - lo + 1)

    def shuffle(self, items):
        for i in range(len(items) - 1, 0, -1):
            j = self.below(i + 1)
            items[i], items[j] = items[j], items[i]
        return items


# ---- strings: UTF-16 unit lists, JV encoding ---------------------------------------------------------------------------
def units_of(text):
    raw = text.encode('utf-16-le')
    return [raw[i] | (raw[i + 1] << 8) for i in range(0, len(raw), 2)]


def jv(units):
    """A unit list as a JV value: a plain string when well-formed UTF-16, else {u16:[...]} (lone surrogates)."""
    try:
        return b''.join(u.to_bytes(2, 'little') for u in units).decode('utf-16-le')
    except UnicodeDecodeError:
        return {'u16': list(units)}


WORDS = ['alpha', 'beta', 'gamma', 'plan', 'fix the bug', 'hello world', 'refactor', 'test', 'x', 'Q', 'deploy', 'notes', 'a-b_c', "it's", '"quoted"', 'tab\there', 'line\nbreak', 'back\\slash']
PIECES = [[0xE9], [0x65, 0x301], [0xD83D, 0xDE00], [0xD835, 0xDCB3], [0xD800], [0xDC00], [0xDBFF], [0xDFFF], [0x85], [0x200B], [0x3000], [0xFEFF], [0xA0], [0x2028],
          [0xAC00, 0xD55C], [0x4E2D, 0x6587], [0x200D], [0x0], [0x1F], [0x7F], [0xFFFD], [0x20], [0x20, 0x20], [0x0A], [0x09]]


def ustr(r, lo=0, hi=12, unicode_pct=60):
    """A random unit list: ASCII words mixed with the awkward pieces (lone surrogates, U+0085, U+200B, astral, combining)."""
    out = []
    for _ in range(r.between(lo, hi)):
        out += PIECES[r.below(len(PIECES))] if r.chance(unicode_pct) else units_of(r.pick(WORDS))
        if r.chance(30): out.append(0x20)
    return out


def uv(r, lo=0, hi=12, unicode_pct=60): return jv(ustr(r, lo, hi, unicode_pct))
def valid_str(r, lo=0, hi=12, unicode_pct=60):
    """Like uv but never a lone surrogate (for places that need a real JSON string, e.g. object keys)."""
    for _ in range(50):
        v = uv(r, lo, hi, unicode_pct)
        if isinstance(v, str): return v
    return 'fallback'


def ascii_id(r, prefix='t'): return f'{prefix}{r.between(0, 6)}'
def hexid(r): return 'acc_' + ''.join(r.pick('0123456789abcdef') for _ in range(16))


# ---- common value builders ---------------------------------------------------------------------------------------------
FOLDERS = ['/proj', '/a/b/proj/', '/', '/p/./x', '/프로젝트/한글', '/p/😀', '/work/é', '/proj/with space', '/a/b/c']
THREADS = ['t1', 't2', 't3', 'th.4', 'x-5', 'T_6', 'a.b-c_9']
HOME = {'claude': 'claude:home', 'codex': 'codex:home', 'grok': 'grok:home', 'opencode': 'opencode:home', 'qwen': 'qwen:home'}


def mk_err(r, ref, kind=None):
    """A random thrown value spec: class, name and code combinations (code only on EngineImportError)."""
    kind = kind or r.pick(['Error', 'TypeError', 'EngineImportError', 'EngineImportError', 'plain', 'primitive', 'abort', 'eie-abort'])
    msg = uv(r, 0, 6)
    if kind == 'primitive':
        return err(ref, 'primitive', value=r.pick(['boom', 0, 7, '', True, valid_str(r, 1, 4)]))
    if kind == 'plain':
        return err(ref, 'plain', name=r.pick(['AbortError', 'TypeError', 'Custom', '']), message=msg)
    if kind == 'abort':
        return err(ref, r.pick(['plain', 'Error', 'TypeError']), name='AbortError', **({'message': msg} if r.chance(50) else {}))
    if kind == 'eie-abort':
        return eie(ref, r.pick(EIE_CODES), name='AbortError', message=msg)
    if kind == 'EngineImportError':
        return eie(ref, r.pick(EIE_CODES), **({'message': msg} if r.chance(60) else {}))
    return err(ref, kind, **({'message': msg} if r.chance(80) else {}))


def mk_turn(r, i, unicode_pct=50, plain_time=False):
    t = {'prompt': uv(r, 0, 8, unicode_pct) if r.chance(85) else '', 'reply': uv(r, 0, 8, unicode_pct), 'tools': r.pick([0, 0, 1, 2, 5, -1, 40]), 'clipped': r.pick([False, False, True])}
    c = r.below(5)
    if c == 1: t['createdAt'] = f'2023-0{r.between(1, 9)}-0{r.between(1, 9)}T0{r.between(0, 9)}:00:00.000Z'
    elif c == 2: t['createdAt'] = f'2023-0{r.between(1, 9)}-0{r.between(1, 9)}T01:00:00.000Z'; t['completedAt'] = f'2023-0{r.between(1, 9)}-1{r.between(0, 9)}T02:00:00.000Z'
    elif c == 3: t['completedAt'] = ISO
    elif c == 4 and not plain_time: t['createdAt'] = r.pick([None, '', {'undef': 1}])
    return t


def mk_record(r, nturns=None, title_mode=None, unicode_pct=50):
    n = r.pick([0, 1, 1, 2, 3, 4, 6, 9]) if nturns is None else nturns
    rec = {'turns': [mk_turn(r, i, unicode_pct) for i in range(n)]}
    tm = title_mode if title_mode is not None else r.below(7)
    if tm == 0: rec['title'] = ''
    elif tm == 1: rec['title'] = 'T'
    elif tm == 2: rec['title'] = jv(units_of(r.pick(['  ', '　', '\t\n', ' ​ ', '\u0085', ' a  b ', 'x\u0085y'])))
    elif tm == 3: rec['title'] = uv(r, 1, 10, 70)
    elif tm == 4: rec['title'] = title80(r)
    elif tm == 5: rec['title'] = uv(r, 20, 40, 40)
    else: rec['title'] = {'undef': 1} if r.chance(50) else None
    if r.chance(30): rec['omittedBefore'] = r.pick([True, False])
    if r.chance(15): rec['model'] = r.pick(['m1', '', valid_str(r, 1, 3)])
    pm = r.below(8)
    if pm == 1: rec['lastPlan'] = uv(r, 1, 6)
    elif pm == 2: rec['lastPlan'] = ''
    elif pm == 3: rec['lastPlan'] = {'steps': [r.between(0, 3), 'a']}
    return rec


def title80(r):
    """Titles around the 80-unit cut: astral or combining characters placed to straddle the boundary."""
    pad = r.between(70, 84)
    units = [0x61 + (i % 20) for i in range(pad)]
    at = r.between(max(0, 77 - r.below(4)), 80)
    units[at:at] = r.pick([[0xD83D, 0xDE00], [0xD835, 0xDCB3], [0x65, 0x301], [0xD83D], [0xDE00], [0x200B], [0x85], [0x20, 0x20, 0x20]])
    if r.chance(30): units = [0x20, 0x20] + units
    return jv(units)


def mk_sessions(r, n, keys, unicode_ids=True):
    """n sessions; keys is a list of (engine, thread) pairs some sessions bind through the three routes. Returns (sessions, bound_keys)."""
    out, bound_keys = [], []
    for i in range(n):
        sid = (uv(r, 1, 5, 40) if unicode_ids and r.chance(30) else f's{i}')
        if r.chance(10) and out: sid = out[r.below(len(out))]['id']       # duplicate session id
        s = sess(sid, uv(r, 0, 6, 50))
        route = r.below(7)
        if keys and route <= 3:
            eng, th = r.pick(keys)
            if route == 0: s['externalThread'] = ext(eng, th)
            elif route == 1: s['externalThread'] = ext(eng, 'orig', forks=[th, 'f2'])
            elif route == 2: s['terminal'] = {'cliSession': th}; s['engine'] = eng if r.chance(70) else None
            else: s['engineThreads'] = slots({r.pick([eng, f'{eng}:{hexid(r)}', f'{eng}:x:y']): th})
            if s.get('engine') is None: s.pop('engine', None)
            bound_keys.append((eng, th))
        elif route == 4: s['engineThreads'] = slots({'claude': ''})
        if r.chance(15): s['temporary'] = r.pick([True, False, None])
        out.append(s)
    return out, bound_keys


def finish(case, extra_errs=()):
    """Collect every thrown-value spec into world.errs, like gen_fixtures.F does."""
    errs = {}
    collect_errs(case['world'], errs)
    collect_errs(case['script'], errs)
    for e in extra_errs: errs[e['ref']] = e
    if errs: case['world']['errs'] = [errs[k] for k in sorted(errs)]
    return case


def world_for(engine, kind, r, rec=None, items=None, copy_entry=None):
    """A world whose reader (and copy) queues serve one op of the engine."""
    if kind == 'list':
        return readers(engine, list=[{'ok': items if items is not None else []}])
    return gworld(rec if rec is not None else mk_record(r)) if engine == 'grok' else okread(engine, rec if rec is not None else mk_record(r))


# ---- families ----------------------------------------------------------------------------------------------------------
def q1(r):
    """sources / projection: accounts, duplicates, env, missing source, label."""
    world, rows_by = {}, {}
    acc = {}
    for eng in ('claude', 'codex'):
        c = r.below(10)
        if c == 0: continue
        if c == 1:
            acc[eng] = {'throw': mk_err(r, f'ae_{eng}')}
            continue
        rows = []
        for i in range(r.pick([0, 1, 1, 2, 3])):
            rid = r.pick([hexid(r), ACC1, ACC2, 'bad', '', 'acc_zz', 'ACC_0123456789ABCDEF', hexid(r)])
            rows.append({'id': rid, 'name': r.pick([None, {'undef': 1}, '', 'Work', uv(r, 1, 6)])})
        if rows and r.chance(20): rows.append(dict(rows[0], name=uv(r, 1, 4)))   # duplicate id
        acc[eng] = {'ok': rows}
        rows_by[eng] = rows
    if acc: world['accounts'] = acc
    if r.chance(40):
        world['env'] = {k: r.pick(['/secret/x', '  ', '', valid_str(r, 1, 5)]) for k in r.pick([['CLAUDE_CONFIG_DIR'], ['GROK_HOME'], ['CLAUDE_CONFIG_DIR', 'GROK_HOME'], ['CODEX_HOME']])}
    if r.chance(15): world['dataDir'] = r.pick(['/data', '/d/ä', '/data/😀'])
    script, n = [], r.between(1, 3)
    for i in range(n):
        req = f'r{i + 1}'
        c = r.below(10)
        if c <= 5: script.append(SRC(req))
        else:
            op = r.pick(['list', 'open'])
            if c <= 7:
                bad = r.pick(['nope:home', '', 'claude:', 'claude:acc_missing', 'claude:home ', 'CLAUDE:home', 'opencode:acc_0123456789abcdef', uv(r, 1, 4)])
                src = bad
            else:
                cands = [f'{e}:{row["id"]}' for e, rows in rows_by.items() for row in rows] + ['claude:home', 'codex:home', 'grok:home']
                src = r.pick(cands)
            script.append((LIST(req, src) if op == 'list' else OPEN(req, src, r.pick(THREADS))))
            eng = src.split(':')[0] if isinstance(src, str) and src.split(':')[0] in ENGINES else 'claude'
            world = merge(world, readers(eng, list=[{'ok': []}], read=[{'ok': mk_record(r, 1)}]), {'copy': [{'ok': {'path': '/grok/copy1', 'skipped': 0}, 'verify': True}]} if eng == 'grok' else {})
    return {'world': world, 'script': script, 'tags': ['sources']}


def q2(r):
    """preflight: folder errors, mode, AI (fromAppAction), Grok account, initial bound."""
    eng = r.pick(['claude', 'claude', 'grok', 'grok', 'codex', 'opencode', 'qwen'])
    op = 'list' if r.chance(20) else 'open'
    mode = r.pick(['fork', 'fork', 'same'])
    thread = r.pick(THREADS)
    folder = r.pick(FOLDERS)
    source = HOME[eng]
    world = {}
    tags = ['preflight']
    if op == 'list': world = merge(world, readers(eng, list=[{'ok': [item(thread, uv(r, 1, 5))]}]))
    else: world = merge(world, world_for(eng, 'open', r))
    fa = r.below(8)
    start_kw = {}
    if fa == 1: start_kw['fromAppAction'] = True
    elif fa == 2: start_kw['fromAppAction'] = False
    elif fa == 3: world['fromAppAction'] = True
    elif fa == 4: world['fromAppAction'] = {'throw': mk_err(r, 'fa')}
    elif fa == 5: world['fromAppAction'] = False
    if r.chance(15): start_kw['remote'] = r.pick([True, False]); start_kw['tempVisible'] = r.pick([True, False])
    target = None
    if eng == 'grok' or r.chance(10):
        ga = r.below(6)
        if ga == 0: world['grokAccounts'] = {'throw': mk_err(r, 'ga')}
        elif ga <= 3: world['grokAccounts'] = [{'id': 'g1'}, {'id': r.pick(['g2', 'g1', 'ゲ'])}]
        target = r.pick([None, 'g1', 'g2', 'nope', '', 'ゲ'])
    fe = r.below(8)
    if fe == 1: world['checkFolder'] = {'*': {'throw': mk_err(r, 'fe')}}
    elif fe == 2 and folder == '/p/./x': world['checkFolder'] = {'/p/./x': {'ok': '/p/x'}}
    elif fe == 3: world['checkFolder'] = {'*': {'throw': err('fe', 'primitive', value=valid_str(r, 1, 4))}}
    elif fe == 4: world['checkFolder'] = {'*': {'throw': err('fe', 'plain', name='AbortError')}}
    if r.chance(15) and eng in ('claude', 'codex'):
        world['accounts'] = {eng: {'ok': [{'id': ACC1, 'name': 'W'}]}}
        if r.chance(70): source = f'{eng}:{ACC1}'
    if r.chance(35):
        sessions, _ = mk_sessions(r, r.between(1, 3), [(eng, thread)], True)
        world['sessions'] = sessions
        if r.chance(25) and sessions: world['isTemporary'] = {sessions[0]['id']: True} if isinstance(sessions[0]['id'], str) else {}
        tags.append('bound')
    script = []
    for i in range(r.between(1, 2)):
        req = f'r{i + 1}'
        if op == 'list': script.append(LIST(req, source, folder, **start_kw))
        else: script.append(OPEN(req, source, thread, folder, mode, target, **start_kw))
    return {'world': world, 'script': script, 'tags': tags}


def q3(r):
    """list: duplicate session ids, titles, temporary, metadata, per-item snapshot."""
    eng = r.pick(['claude', 'claude', 'claude', 'codex', 'grok', 'opencode', 'qwen'])
    n_items = r.pick([0, 1, 2, 3, 4, 5, 7, 12])
    items, keys = [], []
    for i in range(n_items):
        th = r.pick(THREADS + [valid_str(r, 1, 5)] + [uv(r, 1, 4)]) if r.chance(60) else f't{i}'
        it = {'thread': th, 'title': uv(r, 0, 8)}
        if r.chance(25): it['title'] = jv(units_of('x' * r.between(190, 230)))
        for k, vals in (('updatedAt', [0, ISO, 1700000000000, {'undef': 1}, None, '']), ('size', [0, 5, 1024, {'undef': 1}, None]), ('messages', [0, 3, 200, {'undef': 1}, None]),
                        ('path', ['', '/p/f.jsonl', {'undef': 1}, None, uv(r, 1, 4)]), ('entrypoint', ['cli', 'sdk', '', {'undef': 1}]), ('origin', ['lair', 'other', {'undef': 1}]),
                        ('maybeOpen', [True, False, 1, 0, 'yes', {'undef': 1}, None])):
            if r.chance(45): it[k] = r.pick(vals)
        if r.chance(15): it['extra'] = r.pick(['drop me', 5, {'x': 1}])
        items.append(it)
        if isinstance(th, str): keys.append((eng, th))
    n_sess = r.pick([0, 0, 1, 2, 3, 5, 8])
    sessions, bound_keys = mk_sessions(r, n_sess, keys, True)
    world = readers(eng, list=[{'ok': items}])
    if sessions: world['sessions'] = sessions
    if sessions and r.chance(40):
        world['isTemporary'] = {s['id']: r.pick([True, False]) for s in sessions if isinstance(s['id'], str) and r.chance(60)}
    tags = ['list']
    if r.chance(15):
        q = []
        for i in range(r.between(1, n_items + 2)): q.append(None)
        pos = r.below(len(q) + 1)
        e = mk_err(r, 'sl', r.pick(['Error', 'primitive', 'EngineImportError']))
        if pos == len(q): q.append({'throw': e}) if r.chance(50) else q.append({'throw': e, 'throwAt': r.between(0, max(0, len(sessions)))})
        else: q[pos] = {'throw': e} if r.chance(50) else {'throw': e, 'throwAt': r.between(0, max(0, len(sessions)))}
        world['sessionList'] = q
        tags.append('sessionList-throw')
    script = [LIST('r1', HOME[eng], r.pick(FOLDERS))]
    if r.chance(25):
        tags.append('snapshot')
        world['readers'][eng]['list'] = [{'defer': 'ls', 'ok': items}, {'ok': items}]
        removable = [s['id'] for s in sessions if isinstance(s['id'], str)]
        pick = r.below(3)
        if items and pick == 0:
            th = items[r.below(len(items))]['thread']
            script.append({'ev': 'mutate', 'op': 'add', 'session': sess('mut-' + str(r.below(100)), uv(r, 1, 4), externalThread=ext(eng, th))})
        elif pick == 1 and removable:
            script.append({'ev': 'mutate', 'op': 'remove', 'id': r.pick(removable)})
        elif pick == 2 and removable:
            th = items[0]['thread'] if items else 't1'
            script.append({'ev': 'mutate', 'op': 'replace', 'session': sess(r.pick(removable), uv(r, 1, 4), externalThread=ext(eng, th))})
        else:
            script.append({'ev': 'mutate', 'op': 'add', 'session': sess('mut-x', 'M')})
        script.append(SETTLE('ls'))
    if r.chance(15): script.append(LIST('r2', HOME[eng], '/proj'))
    return {'world': world, 'script': script, 'tags': tags}


def q4(r):
    """open data: turns, time, UUID, slot, Unicode, title, plan."""
    eng = r.pick(['claude', 'claude', 'claude', 'codex', 'opencode', 'qwen'])
    mode = r.pick(['fork', 'same', 'fork'])
    rec = mk_record(r, unicode_pct=r.pick([10, 50, 90]))
    if r.chance(15): rec['turns'] = [mk_turn(r, i, 50) for i in range(r.between(10, 30))]
    world = okread(eng, rec)
    tags = ['open-data']
    nu = r.below(6)
    if nu == 1: world['uuids'] = [f'{r.next32():08x}-0000-4000-8000-{r.next32():08x}{r.below(65536):04x}' for _ in range(len(rec['turns']) + 3)]
    elif nu == 2: world['uuids'] = ['dup-uuid'] * (len(rec['turns']) + 2)
    elif nu == 3: world['uuids'] = ['u-' + valid_str(r, 1, 4)] + [{'throw': mk_err(r, 'ue')}] if r.chance(30) else ['a', 'b']
    if r.chance(40): world['clockStart'] = r.pick([0, 1, 1700000000000, 1700000000000.5 if False else 1700000000001, 253402300799000, 86400000])
    if r.chance(30): world['clock'] = {'throw': mk_err(r, 'ce')} if r.chance(25) else None
    if world.get('clock') is None: world.pop('clock', None)
    if r.chance(60):
        slot_pool = {'codex': {'threadId': 'keep'}, 'claude:other': {'threadId': 'o2'}, 'claude': {'threadId': 'old'}, f'{eng}:{ACC1}': {'threadId': 'acct'}, f'{eng}:a:b': {'threadId': ''}}
        picked = {k: v for k, v in slot_pool.items() if r.chance(50)}
        world['newSession'] = {'title': uv(r, 0, 4), 'turns': [], 'engineThreads': picked, 'workflow': {'x': 1}} if r.chance(30) else {'title': '', 'turns': [], 'engineThreads': picked}
        tags.append('slots')
    source = HOME[eng]
    if eng in ('claude', 'codex') and r.chance(25):
        world['accounts'] = {eng: {'ok': [{'id': ACC1, 'name': 'Work'}]}}
        source = f'{eng}:{ACC1}'
    if r.chance(15): world['modelFor'] = [r.pick([{'ok': 'model-x'}, {'ok': ''}, {'ok': {'undef': 1}}])]
    folder = r.pick(FOLDERS)
    script = [OPEN('r1', source, r.pick(THREADS), folder, mode)]
    if r.chance(20): script.append(OPEN('r2', source, 't9', folder, 'fork'))
    return {'world': world, 'script': script, 'tags': tags}


def q5(r):
    """Grok: stage-wise success/failure of target/copy/verify/rollback."""
    world = merge(okread('grok', mk_record(r)), {})
    stage = r.pick(['ok', 'ok', 'home', 'copy', 'verify', 'create', 'create-after', 'race', 'cancel-copy', 'cancel-verify', 'model', 'uuid', 'rm-fail'])
    copy_entry = {'ok': {'path': r.pick(['/grok/copy1', '/g/ö', '/g/p q']), 'skipped': r.pick([0, 0, 2])}, 'verify': True}
    script = []
    tags = ['grok', stage]
    handle = None
    if r.chance(30):
        world['grokAccounts'] = [{'id': 'g1'}, {'id': 'g2'}]
    target = r.pick([None, None, 'g1', 'g2']) if 'grokAccounts' in world else None
    opened = OPEN('r1', 'grok:home', r.pick(THREADS), r.pick(FOLDERS), 'fork', target)
    if stage == 'home': world['grokHomeFor'] = [{'reject': mk_err(r, 'gh')}]
    elif stage == 'copy': copy_entry = {'reject': mk_err(r, 'ce')}
    elif stage == 'verify': world['loadCheck'] = [{'reject': mk_err(r, 've')}]
    elif stage == 'model': world['modelFor'] = [{'throw': mk_err(r, 'me')}]
    elif stage == 'uuid': world['uuids'] = [{'throw': mk_err(r, 'ue')}]
    elif stage in ('create', 'create-after'):
        world['createImported'] = [{'mode': 'fail-before-fill' if stage == 'create' else 'fail-after-fill', 'throw': mk_err(r, 'cr')}]
        if r.chance(40): world['rm'] = [{'reject': mk_err(r, 'rm')}]
    if stage == 'rm-fail':
        world['createImported'] = [{'mode': 'fail-before-fill', 'throw': mk_err(r, 'cr')}]
        world['rm'] = [r.pick([{'reject': mk_err(r, 'rm')}, {'ok': None}])]
    if stage == 'race':
        copy_entry = dict(copy_entry, defer='cp')
        bound = ext('grok', opened['input']['thread'])
        if r.chance(40): world['rm'] = [{'reject': mk_err(r, 'rm')}]
        script = [opened, {'ev': 'mutate', 'op': 'add', 'session': sess('raced', uv(r, 0, 4), externalThread=bound)}, SETTLE('cp')]
    elif stage == 'cancel-copy':
        copy_entry = dict(copy_entry, defer='cp')
        reason = r.pick([None, mk_err(r, 'R', 'Error'), err('R', 'primitive', value='why')])
        script = [{'ev': 'handle', 'name': 'c1'}, dict(opened, cancel='c1'), CANCEL('c1', reason), SETTLE('cp')]
    elif stage == 'cancel-verify':
        world['loadCheck'] = [{'defer': 'lc', 'onAbort': abort_err()}]
        script = [{'ev': 'handle', 'name': 'c1'}, dict(opened, cancel='c1'), CANCEL('c1', r.pick([None, mk_err(r, 'R', 'Error')]))]
    else:
        script = [opened]
        if r.chance(15): script.append(OPEN('r2', 'grok:home', 't8', '/proj', 'fork'))
    world['copy'] = [copy_entry]
    if stage == 'cancel-verify' and r.chance(40):
        script = [{'ev': 'handle', 'name': 'c1'}, dict(opened, cancel='c1'), ADV(r.pick([20000, 25000]))]
        if r.chance(30): script = script[:-1] + [ADV(19999), ADV(1)]
        tags.append('deadline')
    return {'world': world, 'script': script, 'tags': tags}


def q6(r):
    """errors / cancel: class/name/code combos, reason identity, deadline, cancel during cleanup."""
    scenario = r.pick(['reject', 'reject', 'reject', 'cancel-read', 'cancel-pre', 'deadline', 'deadline-ignored', 'cancel-race-rm', 'cancel-catch-rm', 'cancel-list', 'codex-deadline', 'reason-vs-abort'])
    tags = ['errors', scenario]
    eng = r.pick(['claude', 'claude', 'codex', 'opencode', 'qwen'])
    rec = mk_record(r, 2)
    inp = lambda req, e=None, **kw: OPEN(req, HOME[e or eng], r.pick(THREADS), '/proj', 'fork', **kw)
    if scenario == 'reject':
        e = mk_err(r, 'e1')
        if r.chance(25):
            op = r.pick(['list', 'open'])
            world = readers(eng, list=[{'reject': e}], read=[{'reject': e}])
            script = [LIST('r1', HOME[eng]) if op == 'list' else inp('r1')]
        else:
            first = r.chance(35)
            world = readers(eng, read=[{'reject': e}, {'ok': rec}] if first else [{'reject': e}])
            script = [inp('r1'), inp('r2')] if first else [inp('r1')]
            if r.chance(30) and eng != 'claude': world = merge(world, {'makeCodexRunDeps': [{'reject': mk_err(r, 'mk')}]}) if eng == 'codex' else merge(world, {'locateAcp': [{'reject': mk_err(r, 'lo')}]})
        return {'world': world, 'script': script, 'tags': tags}
    world = readers(eng, read=[{'defer': 'rd', 'ok': rec, 'onAbort': abort_err()}], list=[{'defer': 'rd', 'ok': [], 'onAbort': abort_err()}])
    reason = r.pick([None, None, mk_err(r, 'R', 'Error'), err('R', 'primitive', value=r.pick(['stop', 0, ''])), err('R', 'plain', name='AbortError'), eie('R', r.pick(EIE_CODES))])
    if scenario == 'cancel-read':
        script = [{'ev': 'handle', 'name': 'c1'}, inp('r1', cancel='c1'), CANCEL('c1', reason)]
        if r.chance(40): script.append(SETTLE('rd')) if False else None
        script = [s for s in script if s]
    elif scenario == 'cancel-pre':
        world = merge(world, okread(eng, rec))
        script = [{'ev': 'handle', 'name': 'c1'}, CANCEL('c1', reason), inp('r1', cancel='c1')]
    elif scenario == 'cancel-list':
        script = [{'ev': 'handle', 'name': 'c1'}, dict(LIST('r1', HOME[eng]), cancel='c1'), CANCEL('c1', reason), SETTLE('rd')]
    elif scenario in ('deadline', 'deadline-ignored', 'codex-deadline'):
        ms = r.pick([19999, 20000, 20001, 5000, 40000])
        if scenario == 'deadline-ignored': world = readers(eng, read=[{'defer': 'rd', 'ok': rec}])
        if scenario == 'codex-deadline':
            world = merge(readers('codex', read=[{'ok': rec}]), {'makeCodexRunDeps': [{'ok': {'ref': 'codexRun'}, 'defer': 'mk'}]})
            script = [OPEN('r1', 'codex:home', 't1'), ADV(r.pick([20000, 19999, 1])), SETTLE('mk')]
            if ms == 19999: script.insert(2, ADV(1))
            return {'world': world, 'script': script, 'tags': tags}
        script = [inp('r1')]
        if ms <= 20000 and r.chance(40) and ms != 20000:
            script += [ADV(ms), ADV(20000 - ms)]
        else: script.append(ADV(ms))
        if scenario == 'deadline-ignored' or ms < 20000 and r.chance(30) and False: script.append(SETTLE('rd'))
        if scenario == 'deadline-ignored': pass
        elif ms < 20000 and (not script[-1].get('ms') or sum(s.get('ms', 0) for s in script) < 20000):
            script.append(SETTLE('rd'))
    elif scenario == 'reason-vs-abort':
        script = [{'ev': 'handle', 'name': 'c1'}, inp('r1', cancel='c1'), CANCEL('c1', reason or mk_err(r, 'R', 'Error')), ADV(20000)]
    else:   # cancel during cleanup (Grok race rm or catch rm)
        grok_rec = mk_record(r, 1)
        world = gworld(grok_rec)
        opened = OPEN('r1', 'grok:home', 't1', '/proj', 'fork')
        if scenario == 'cancel-race-rm':
            world['copy'] = [dict(world['copy'][0], defer='cp')]
            world['rm'] = [r.pick([{'defer': 'rmd'}, {'defer': 'rmd', 'reject': None} if False else {'defer': 'rmd'}])]
            script = [{'ev': 'handle', 'name': 'c1'}, dict(opened, cancel='c1'), {'ev': 'mutate', 'op': 'add', 'session': sess('s9', '', externalThread=ext('grok', opened['input']['thread']))}, SETTLE('cp'),
                      CANCEL('c1', reason), SETTLE('rmd')]
        else:
            world['createImported'] = [{'mode': 'fail-before-fill', 'throw': mk_err(r, 'cr', 'Error')}]
            world['rm'] = [{'defer': 'rmd'}]
            script = [{'ev': 'handle', 'name': 'c1'}, dict(opened, cancel='c1'), CANCEL('c1', reason), SETTLE('rmd')]
    return {'world': world, 'script': script, 'tags': tags}


def q7(r, tie):
    """concurrency: 2-4 requests on one instance, same/other key, session mutation, retries."""
    eng = 'claude'
    n = r.between(2, 4)
    tags = ['concurrency']
    sessions = []
    if r.chance(15): sessions = [bound_session := sess('pre', 'Pre', externalThread=ext(eng, r.pick(THREADS[:3])))]
    bound_keys = {s['externalThread']['thread'] for s in sessions}
    pool = THREADS[:3] if r.chance(70) else THREADS[:2]
    # first wave: n starts with distinct deferred reader labels; each start reads iff not bound and key not in flight
    script, entries, labels, inflight, reqs = [], [], [], set(), []
    seq = 0
    for i in range(n):
        th = r.pick(pool)
        folder = r.pick(['/proj', '/proj', '/other'])
        source = r.pick(['claude:home', 'claude:home', f'claude:{ACC1}'])
        req = f'r{i + 1}'
        script.append(OPEN(req, source, th, folder, 'fork'))
        reads = th not in bound_keys and th not in inflight
        if reads:
            label = f'L{len(labels) + 1}'
            labels.append((label, req, th))
            e = {'defer': label, 'onAbort': abort_err()} if tie else {'defer': label}
            entries.append(dict(e, **({'reject': mk_err(r, f'e{len(labels)}')} if False else {'ok': mk_record(r, r.between(0, 3))})))
            inflight.add(th)
        reqs.append((req, th, reads))
        if i < n - 1 and not tie: script.append(ADV(1))
        if r.chance(20):                                                         # mutation of the session list mid-flight
            mth = r.pick(pool)
            script.append({'ev': 'mutate', 'op': 'add', 'session': sess(f'm{i}', uv(r, 0, 3), externalThread=ext(eng, mth))})
            bound_keys.add(mth)
            tags.append('mutate')
    world = {'accounts': {'claude': {'ok': [{'id': ACC1, 'name': 'W'}]}}}
    if sessions: world['sessions'] = sessions
    tail = {'ok': mk_record(r, 1)} if r.chance(70) else {'reject': mk_err(r, 'et', r.pick(['EngineImportError', 'Error']))}
    world = merge(world, readers(eng, read=[*entries, tail] if entries else [tail]))
    if tie:
        tags.append('tie')
        script.append(ADV(20000))
        for label, _, _ in labels: pass                                         # abort fires both deadlines at one instant; onAbort rejects each read
    else:
        order = r.shuffle(list(range(len(labels))))
        for k in order:
            lab = labels[k][0]
            if r.chance(25):
                script.append(SETTLE(lab, reject=mk_err(r, f'sr{k}', r.pick(['EngineImportError', 'Error', 'plain']))))
            else: script.append(SETTLE(lab))
            script.append(ADV(1))
    if not tie and r.chance(45):                                              # retry wave once everything is settled
        tags.append('retry')
        for j in range(r.between(1, 2)):
            script.append(OPEN(f'x{j + 1}', 'claude:home', r.pick(pool), '/proj', 'fork'))
            script.append(ADV(1))
    return {'world': world, 'script': script, 'tags': tags}


def q8(r):
    """protocol/domain: an honest scenario plus the `corrupt` descriptor the implementation applies later (oracle runs it honestly)."""
    kind = r.pick(['completion', 'completion', 'limit', 'excluded'])
    field_pool = {'completion': [('reader', 'value'), ('reader', 'kind'), ('copy', 'path'), ('copy', 'skipped'), ('loadCheck', 'value'), ('createImported', 'id'), ('rm', 'value'),
                                 ('uuid', 'value'), ('now', 'value'), ('sessionList', 'item'), ('checkFolder', 'path'), ('fromAppAction', 'value'), ('timer', 'id'), ('settle', 'label')],
                  'limit': [('turns', 'count'), ('items', 'count'), ('title', 'units'), ('string', 'units'), ('sessions', 'count'), ('forks', 'count')],
                  'excluded': [('number', 'NaN'), ('number', '-0'), ('number', 'Infinity'), ('string', 'lone-surrogate-key'), ('object', 'proto-key'), ('array', 'sparse')]}
    where, field = r.pick(field_pool[kind])
    tags = ['domain', kind]
    base = r.pick(['open', 'open', 'list', 'grok', 'sources', 'cancel'])
    if kind == 'limit':
        if where == 'turns' or base == 'open': base = 'open'
        if where == 'items': base = 'list'
    if base == 'sources': case = q1(r)
    elif base == 'list':
        case = q3(r)
        if kind == 'limit' and where == 'items':
            eng = r.pick(['claude', 'codex'])
            n_items = r.pick([49, 50, 51, 52])
            lst = [{'thread': f't{i}', 'title': uv(r, 0, 3)} for i in range(n_items)]
            case['world'] = readers(eng, list=[{'ok': lst}]); case['script'] = [LIST('r1', HOME[eng])]
    elif base == 'grok': case = q5(r)
    elif base == 'cancel': case = q6(r)
    else:
        case = q4(r)
        if kind == 'limit' and where in ('turns', 'title', 'string'):
            eng = r.pick(['claude', 'codex'])
            rec = mk_record(r, r.pick([199, 200, 201, 205]) if where == 'turns' else r.pick([1, 3]), title_mode=4 if where == 'title' else None, unicode_pct=20)
            if where == 'string': rec['turns'][0]['reply'] = jv(units_of('y' * r.pick([32767, 32768, 32769]) )) if rec['turns'] else None
            case['world'] = okread(eng, rec); case['script'] = [OPEN('r1', HOME[eng], 't1')]
        if where == 'forks' or where == 'sessions':
            sessions, _ = mk_sessions(r, r.pick([20, 60]), [('claude', 't1')], True)
            case['world']['sessions'] = sessions
    case['tags'] = tags + [t for t in case.get('tags', []) if t not in tags]
    case['corrupt'] = {'kind': kind, 'at': where, 'field': field, 'nth': r.below(4)}
    return case


# ---- driver -----------------------------------------------------------------------------------------------------------
def generate(seed=SEED):
    r = Rng(seed)
    multiset = [fam for fam, n in FAMILIES for _ in range(n)]
    r.shuffle(multiset)
    counters = {fam: 0 for fam, _ in FAMILIES}
    tie_left = 12                                                                 # <= ~5% of Q7
    cases = []
    for fam in multiset:
        counters[fam] += 1
        cid = f'{fam}-{counters[fam]:04d}'
        if fam == 'Q7':
            tie = tie_left > 0 and r.chance(5)
            if tie: tie_left -= 1
            body = q7(r, tie)
            if tie: body['tieBreak'] = True
        else:
            body = {'Q1': q1, 'Q2': q2, 'Q3': q3, 'Q4': q4, 'Q5': q5, 'Q6': q6, 'Q8': q8}[fam](r)
        case = {'id': cid, 'family': fam, 'world': body.pop('world'), 'script': body.pop('script'), **body}
        finish(case)
        cases.append(case)
    return cases


def main(argv):
    out = Path(argv[argv.index('--out') + 1]) if '--out' in argv else HERE / 'work' / 'q' / 'cases.jsonl'
    seed = int(argv[argv.index('--seed') + 1]) if '--seed' in argv else SEED
    cases = generate(seed)
    out.parent.mkdir(parents=True, exist_ok=True)
    with open(out, 'w', encoding='utf-8') as f:
        for c in cases: f.write(json.dumps(c, ensure_ascii=True, separators=(',', ':')) + '\n')
    from collections import Counter
    print(f'wrote {len(cases)} cases to {out} (seed {seed}): ' + ' '.join(f'{k}={v}' for k, v in sorted(Counter(c['family'] for c in cases).items())))
    print(f"tieBreak: {sum(1 for c in cases if c.get('tieBreak'))} of {sum(1 for c in cases if c['family'] == 'Q7')} Q7 cases")


if __name__ == '__main__':
    main(sys.argv)
