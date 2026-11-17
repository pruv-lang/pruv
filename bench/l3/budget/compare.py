"""L3-10 request budget: observation comparator (spec 7.5, 11, 15.1) and the JV codec.

  compare(expected_obs, actual_obs) -> (cls, detail)       cls in CLASSES, detail = '' when exact, else the first difference
  jv_decode(v) / jv_encode(x)                              JV <-> python values (UNDEF is the sentinel for {undef:1})
  python3 compare.py --selftest                            codec round trip + comparator mutation checks, exit 0/1

Classes
  exact            the whole observation is equal (all keys, trace order, finalSessions/Clocks/ToolSeen, timers, driver)
  P6               (a) the only differences are the message text of thrown values whose error name is equal
                       (requests[i].value.error.message, terminal thrown on both sides); this is the task's definition.
                   (b) spec 11 defines P6 as a "named resource bound with approved prefix/state behavior" and 8.4 maps
                       ResourceBound(input) to it: the actual run ends a request with terminal 'core' and code
                       ResourceBound/InputTooLarge, every request and trace record before that request is equal to the oracle.
                       (ResourceBound for a number is excluded-domain, spec 8.4.) Both readings are accepted; the spec wins
                       where they would conflict (nothing conflicts today).
  excluded-domain  expected.excluded is non-empty or expected.domain is set (spec 2.3, 15.1 #10/#16/#26/#27): reported,
                   never counted as an exact success.
  unexplained      everything else, including a missing key, a malformed or crashed implementation line, driver violations
                   in the actual observation and InternalInvariant.

KD-2 (spec 15.1 #2): inside the trace records of a request whose op is replaceForUserMessage, `changed` records may move
relative to the other records of that request. The comparison drops them from both traces and requires an equal count of
`changed` records per such request.
KD-1 needs no normalisation (the oracle clock already returns one value per call).

The comparison is type strict (null, false, 0, 0.0, "" and a missing key all differ). Plain JSON object key order carries no
meaning; the ordered encoding is {"obj":[[k,v],...]}, which is compared as an ordered array."""
import sys
sys.dont_write_bytecode = True
import copy
import json
import re
import struct

CLASSES = ('exact', 'P6', 'excluded-domain', 'unexplained')
MAX_DIFFS = 50
MISSING = '<missing>'


class _Undef:
    """Sentinel for the JS value undefined ({undef:1})."""
    _inst = None

    def __new__(cls):
        if cls._inst is None:
            cls._inst = super().__new__(cls)
        return cls._inst

    def __repr__(self):
        return 'UNDEF'

    def __bool__(self):
        return False


UNDEF = _Undef()
MARKERS = ('u16', 'undef', 'num', 'arr', 'obj')
SAFE = 2 ** 53 - 1


# ---- JV codec ----------------------------------------------------------------------------------------------------------------
def _units_to_str(units):
    raw = b''.join(struct.pack('<H', u) for u in units)
    return raw.decode('utf-16-le', errors='surrogatepass')


def _str_to_units(s):
    raw = s.encode('utf-16-le', errors='surrogatepass')
    return [struct.unpack_from('<H', raw, i)[0] for i in range(0, len(raw), 2)]


def jv_decode(v):
    """JV -> python. u16 -> str (surrogate pairs joined, lone surrogates kept), num -> float, obj -> dict in order, arr/list -> list,
    undef -> UNDEF. Other dicts (e.g. {sref, v}, {bref, v}, {error:{...}}) are decoded key by key."""
    if isinstance(v, dict):
        if len(v) == 1:
            (k, x), = v.items()
            if k == 'u16' and isinstance(x, list):
                return _units_to_str(x)
            if k == 'undef':
                return UNDEF
            if k == 'num' and isinstance(x, str) and re.fullmatch(r'[0-9a-fA-F]{16}', x):
                return struct.unpack('>d', bytes.fromhex(x))[0]
            if k == 'arr' and isinstance(x, list):
                return [jv_decode(e) for e in x]
            if k == 'obj' and isinstance(x, list):
                out = {}
                for pair in x:
                    out[jv_decode(pair[0])] = jv_decode(pair[1])
                return out
        return {key: jv_decode(x) for key, x in v.items()}
    if isinstance(v, list):
        return [jv_decode(e) for e in v]
    return v


def _enc_str(s):
    try:
        s.encode('utf-8')
        return s
    except UnicodeEncodeError:
        return {'u16': _str_to_units(s)}


def jv_encode(x):
    """python -> canonical JV as the oracle writes it: safe integers plain (not -0), other numbers {num}, undefined {undef:1},
    ill-formed strings {u16}, dicts {obj:[[k,v]...]}, lists plain."""
    if x is UNDEF:
        return {'undef': 1}
    if x is None or isinstance(x, bool):
        return x
    if isinstance(x, int):
        return x if abs(x) <= SAFE else {'num': struct.pack('>d', float(x)).hex()}
    if isinstance(x, float):
        if x.is_integer() and abs(x) <= SAFE and not (x == 0 and str(x).startswith('-')):
            return int(x)
        return {'num': struct.pack('>d', x).hex()}
    if isinstance(x, str):
        return _enc_str(x)
    if isinstance(x, (list, tuple)):
        return [jv_encode(e) for e in x]
    if isinstance(x, dict):
        return {'obj': [[_enc_str(k) if isinstance(k, str) else jv_encode(k), jv_encode(e)] for k, e in x.items()]}
    raise TypeError(f'cannot JV-encode {type(x).__name__}')


# ---- structural diff ---------------------------------------------------------------------------------------------------------
def _tag(v):
    if v is None:
        return 'null'
    if isinstance(v, bool):
        return 'bool'
    if isinstance(v, int):
        return 'int'
    if isinstance(v, float):
        return 'float'
    if isinstance(v, str):
        return 'str'
    if isinstance(v, list):
        return 'list'
    if isinstance(v, dict):
        return 'dict'
    return type(v).__name__


def _key(k):
    return '.' + k if re.fullmatch(r'[A-Za-z_$][A-Za-z0-9_$]*', k) else '[%s]' % json.dumps(k, ensure_ascii=False)


def _diff(e, a, path, out):
    """Appends (path, expected, actual) for every difference (stops at MAX_DIFFS). Dict keys are visited in sorted order."""
    if len(out) >= MAX_DIFFS:
        return
    te, ta = _tag(e), _tag(a)
    if te != ta:
        out.append((path, e, a))
    elif te == 'dict':
        for k in sorted(e):
            if k not in a:
                out.append((path + _key(k), e[k], MISSING))
            else:
                _diff(e[k], a[k], path + _key(k), out)
        for k in sorted(a):
            if k not in e:
                out.append((path + _key(k), MISSING, a[k]))
    elif te == 'list':
        n = min(len(e), len(a))
        for i in range(n):
            _diff(e[i], a[i], '%s[%d]' % (path, i), out)
        for i in range(n, max(len(e), len(a))):
            out.append(('%s[%d]' % (path, i), e[i] if i < len(e) else MISSING, a[i] if i < len(a) else MISSING))
    elif e != a:
        out.append((path, e, a))


def diff_observations(expected, actual):
    out = []
    _diff(expected, actual, '', out)
    return out


def _short(v, n=160):
    s = json.dumps(v, ensure_ascii=False) if not isinstance(v, str) else v
    return s if len(s) <= n else s[:n] + '...'


def _fmt(d):
    return '%s: expected %s, got %s' % (d[0].lstrip('.') or '<root>', _short(d[1]), _short(d[2]))


# ---- KD-2 normalisation ------------------------------------------------------------------------------------------------------
def _replace_ids(obs):
    return {r.get('id') for r in obs.get('requests', []) if isinstance(r, dict) and r.get('op') == 'replaceForUserMessage'}


def _changed_counts(obs, ids):
    counts = {}
    for t in obs.get('trace', []):
        if isinstance(t, dict) and t.get('c') == 'changed' and t.get('r') in ids:
            counts[t['r']] = counts.get(t['r'], 0) + 1
    return counts


def _strip_changed(obs, ids):
    out = dict(obs)
    out['trace'] = [t for t in obs.get('trace', []) if not (isinstance(t, dict) and t.get('c') == 'changed' and t.get('r') in ids)]
    return out


# ---- classification ----------------------------------------------------------------------------------------------------------
_MSG = re.compile(r'^requests\[(\d+)\]\.value\.error\.message$')


def _p6_messages(diffs, expected, actual):
    """P6 (a): all differences are error.message of a thrown request pair with the same error name."""
    if not diffs:
        return False
    for path, _e, _a in diffs:
        m = _MSG.match(path.lstrip('.'))
        if not m:
            return False
        i = int(m.group(1))
        try:
            re_, ra = expected['requests'][i], actual['requests'][i]
        except (KeyError, IndexError, TypeError):
            return False
        if re_.get('terminal') != 'thrown' or ra.get('terminal') != 'thrown':
            return False
        if re_['value']['error'].get('name') != ra['value']['error'].get('name'):
            return False
    return True


def _core_bound(actual):
    """-> (index, kind) of the first request ended by a resource bound ('input' | 'number' | 'other'), else None."""
    for i, r in enumerate(actual.get('requests', [])):
        if isinstance(r, dict) and r.get('terminal') == 'core':
            text = json.dumps(r.get('value'), ensure_ascii=False)
            if 'ResourceBound' in text or 'InputTooLarge' in text:
                return i, ('number' if 'number' in text else 'input' if 'input' in text or 'InputTooLarge' in text else 'other')
    return None


def _p6_bound(expected, actual, ids):
    """P6 (b): requests and trace records of every request before the bounded one are equal to the oracle."""
    hit = _core_bound(actual)
    if hit is None:
        return None
    k, kind = hit
    if kind == 'number':
        return 'excluded-domain'
    exp_pre, act_pre = expected.get('requests', [])[:k], actual.get('requests', [])[:k]
    if diff_observations(exp_pre, act_pre):
        return None
    prior = {r.get('id') for r in exp_pre}
    ets = [t for t in _strip_changed(expected, ids).get('trace', []) if t.get('r') in prior]
    ats = [t for t in _strip_changed(actual, ids).get('trace', []) if t.get('r') in prior]
    return 'P6' if not diff_observations(ets, ats) else None


def compare(expected, actual):
    """-> (cls, detail). See the module docstring."""
    if isinstance(expected, dict) and (expected.get('excluded') or expected.get('domain') is not None):
        names = expected.get('excluded') or []
        why = 'excluded: ' + ', '.join(map(str, names)) if names else 'domain: ' + str(expected.get('domain'))
        return 'excluded-domain', why
    if not isinstance(actual, dict) or 'requests' not in actual or 'trace' not in actual:
        return 'unexplained', 'malformed actual observation: ' + _short(actual, 120)
    if expected.get('id') != actual.get('id'):
        return 'unexplained', 'id: expected %s, got %s' % (_short(expected.get('id')), _short(actual.get('id')))
    if 'InternalInvariant' in json.dumps(actual, ensure_ascii=False) and 'InternalInvariant' not in json.dumps(expected, ensure_ascii=False):
        return 'unexplained', 'actual contains InternalInvariant'
    ids = _replace_ids(expected) | _replace_ids(actual)
    ce, ca = _changed_counts(expected, ids), _changed_counts(actual, ids)
    count_diff = None
    if ce != ca:
        rid = next(r for r in sorted(set(ce) | set(ca), key=str) if ce.get(r, 0) != ca.get(r, 0))
        count_diff = 'trace: KD-2 changed count for request %s: expected %d, got %d' % (rid, ce.get(rid, 0), ca.get(rid, 0))
    exp_n, act_n = _strip_changed(expected, ids), _strip_changed(actual, ids)
    diffs = diff_observations(exp_n, act_n)
    if not diffs and count_diff is None:
        return 'exact', ''
    if count_diff is not None:
        return 'unexplained', count_diff + ('; also ' + _fmt(diffs[0]) if diffs else '')
    if _p6_messages(diffs, expected, actual):
        return 'P6', _fmt(diffs[0])
    bound = _p6_bound(expected, actual, ids)
    if bound is not None:
        return bound, _fmt(diffs[0])
    return 'unexplained', _fmt(diffs[0])


# ---- self test ---------------------------------------------------------------------------------------------------------------
class _Xor:
    """xorshift32 (13, 17, 5)."""

    def __init__(self, seed):
        self.s = seed & 0xFFFFFFFF or 1

    def next(self):
        x = self.s
        x ^= (x << 13) & 0xFFFFFFFF
        x ^= x >> 17
        x ^= (x << 5) & 0xFFFFFFFF
        self.s = x
        return x

    def below(self, n):
        limit = 0x100000000 - 0x100000000 % n  # rejection sampling
        while True:
            x = self.next()
            if x < limit:
                return x % n

    def pick(self, items):
        return items[self.below(len(items))]


_POOL = ['a', 'Z', '0', ' ', 'é', '́', '한', '😀', '\u0085', '​', '"', '\\', '\n']
_NUMS = [0.5, -0.0, float('nan'), float('inf'), float('-inf'), 2.0 ** 60, 2.0 ** 53, -(2.0 ** 53), 1e-300, -3.25]


def _gen_str(r):
    if r.below(4) == 0:  # a string with one genuinely lone surrogate -> {u16}
        lone = 0xD800 + r.below(0x800)
        pre = [ord('p')] * r.below(3)
        post = [ord('q')] * r.below(3)
        return {'u16': pre + [lone] + post}
    return ''.join(r.pick(_POOL) for _ in range(r.below(5)))


def _gen_jv(r, depth=0):
    kind = r.below(10 if depth < 3 else 7)
    if kind == 0:
        return None
    if kind == 1:
        return r.below(2) == 0
    if kind == 2:
        return r.pick([0, 1, -1, 300, SAFE, -SAFE, r.below(100000)])
    if kind in (3, 4):
        return _gen_str(r)
    if kind == 5:
        return {'undef': 1}
    if kind == 6:
        return {'num': struct.pack('>d', r.pick(_NUMS)).hex()}
    if kind in (7, 8):
        return [_gen_jv(r, depth + 1) for _ in range(r.below(4))]
    return {'obj': [[_gen_str(r) if i == 0 else 'k%d' % i, _gen_jv(r, depth + 1)] for i in range(r.below(4))]}


def _fix_keys(v):
    """Make {obj} keys unique (the generator can repeat a key through _gen_str); keys become k<i>."""
    if isinstance(v, list):
        return [_fix_keys(x) for x in v]
    if isinstance(v, dict) and set(v) == {'obj'}:
        return {'obj': [['k%d' % i, _fix_keys(p[1])] for i, p in enumerate(v['obj'])]}
    return v


def _obs(value):
    return {
        'id': 'T1', 'excluded': [],
        'requests': [{'id': 'r1', 'instance': 'A', 'op': 'charge', 'terminal': 'ok', 'value': value}],
        'trace': [{'c': 'call.start', 'r': 'r1', 'i': 'A', 'a': value}, {'c': 'now', 'r': 'r1', 'i': 'A', 'o': value}, {'c': 'call.end', 'r': 'r1', 'i': 'A'}],
        'finalSessions': [{'sref': 's1', 'v': {'obj': [['id', 's1']]}}], 'finalClocks': {'A': {'obj': []}}, 'finalToolSeen': {'A': {'obj': []}},
        'activeTimers': [], 'driver': None,
    }


def selftest():
    r = _Xor(20261117)
    problems = []
    for n in range(300):
        v = _fix_keys(_gen_jv(r))
        d = jv_decode(v)
        e = jv_encode(d)
        if json.dumps(e, sort_keys=True) != json.dumps(v, sort_keys=True):
            problems.append(f'round trip {n}: {_short(v)} -> {_short(e)}')
            continue
        o = _obs(v)
        if compare(o, copy.deepcopy(o))[0] != 'exact':
            problems.append(f'case {n}: identical observations not exact')
            continue
        muts = []
        m = copy.deepcopy(o)
        m['requests'][0]['value'] = {'obj': [['x', v]]}
        muts.append(('value', m))
        m = copy.deepcopy(o)
        m['trace'].append({'c': 'extra', 'r': 'r1', 'i': 'A'})
        muts.append(('trace append', m))
        m = copy.deepcopy(o)
        m['trace'].pop(1)
        muts.append(('trace drop', m))
        m = copy.deepcopy(o)
        m['trace'][1]['o'] = {'obj': [['y', v]]}
        muts.append(('trace o', m))
        m = copy.deepcopy(o)
        m['requests'][0]['terminal'] = 'thrown'
        muts.append(('terminal', m))
        m = copy.deepcopy(o)
        m['activeTimers'].append({'token': 't1', 'delay': 1, 'due': 1, 'i': 'A'})
        muts.append(('timer', m))
        m = copy.deepcopy(o)
        m['driver'] = {'violations': ['x']}
        muts.append(('driver', m))
        for label, mut in muts:
            if compare(o, mut)[0] != 'unexplained':
                problems.append(f'case {n}: mutation {label} not detected')
    # classes
    base = _obs(1)
    ex = copy.deepcopy(base)
    ex['domain'] = 'smoke'
    if compare(ex, base)[0] != 'excluded-domain':
        problems.append('excluded-domain (domain) not reported')
    ex = copy.deepcopy(base)
    ex['excluded'] = ['proxy']
    if compare(ex, base)[0] != 'excluded-domain':
        problems.append('excluded-domain (excluded) not reported')
    thr = _obs(None)
    thr['requests'][0].update(terminal='thrown', value={'error': {'name': 'TypeError', 'message': 'a'}})
    other = copy.deepcopy(thr)
    other['requests'][0]['value']['error']['message'] = 'b'
    if compare(thr, other)[0] != 'P6':
        problems.append('P6 message difference not classified')
    other['requests'][0]['value']['error']['name'] = 'RangeError'
    if compare(thr, other)[0] != 'unexplained':
        problems.append('P6 must not cover a different error name')
    # KD-2
    rep = _obs(1)
    rep['requests'][0]['op'] = 'replaceForUserMessage'
    rep['trace'] = [{'c': 'a', 'r': 'r1', 'i': 'A'}, {'c': 'changed', 'r': 'r1', 'i': 'A'}, {'c': 'b', 'r': 'r1', 'i': 'A'}]
    moved = copy.deepcopy(rep)
    moved['trace'] = [moved['trace'][0], moved['trace'][2], moved['trace'][1]]
    if compare(rep, moved)[0] != 'exact':
        problems.append('KD-2: moved changed record not normalised')
    gone = copy.deepcopy(rep)
    del gone['trace'][1]
    if compare(rep, gone)[0] != 'unexplained':
        problems.append('KD-2: missing changed record not detected')
    twice = copy.deepcopy(rep)
    twice['trace'].append({'c': 'changed', 'r': 'r1', 'i': 'A'})
    if compare(rep, twice)[0] != 'unexplained':
        problems.append('KD-2: extra changed record not detected')
    plain_op = copy.deepcopy(rep)
    plain_op['requests'][0]['op'] = 'attach'
    moved_plain = copy.deepcopy(plain_op)
    moved_plain['trace'] = moved['trace']
    if compare(plain_op, moved_plain)[0] != 'unexplained':
        problems.append('KD-2 must only apply to replaceForUserMessage')
    if jv_decode({'undef': 1}) is not UNDEF or jv_decode({'u16': [0xD83D, 0xDE00]}) != '\U0001F600':
        problems.append('decode of undef / surrogate pair')
    print('compare selftest: 300 JV round trips, ~2100 mutations, class checks, %d problems' % len(problems))
    for p in problems[:20]:
        print('  ' + p)
    return 0 if not problems else 1


if __name__ == '__main__':
    if '--selftest' in sys.argv:
        sys.exit(selftest())
    print(__doc__)
    sys.exit(2)
