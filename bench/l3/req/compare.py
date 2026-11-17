"""L3-9 observation comparator (ABI 12.1, specification 7.3).

  compare_observation(expected, actual, policy=None) -> {class, path, expected, actual}
  compare_batch(expected_list, actual_list, policies=None) -> {results, counts, failures, ok}
  default_policy(fixture_id) -> policy or None   (P08: explicit 32-active-request P6 policy)

Classes: exact, P5, P6, engine-policy, excluded-domain, unexplained.

The comparison is over the complete observation (id, requests, trace, createdSessions, openingEvents, finalOpening,
activeTimers, activeListeners, driver). It is strict about JSON types (missing key, {"undef":1}, null, false, 0, 0.0, "" are all
different), keeps array and {"obj":[[k,v],...]} entry order, and normalizes only opaque reference names ({"ref":name} anywhere,
one shared first-appearance map per observation). Plain JSON object key order carries no meaning (serializers differ); the
ordered object encoding is the {"obj"} array. Paths are stable: `trace[17].a.options.signal`, `requests[0].value.native.message`.

Policy (dict, every key optional):
  P5: {paths: [pattern,...]}          only these explicitly listed native diagnostic prose paths may differ (both sides strings /
                                      {u16}); pattern uses `[*]` for any index; a listed path covers its subtree.
  P6: {trigger: {name, limit[, reduced]}, mustTrigger?, requiredCleanup?}
                                      named bound trigger: active-requests (32), packet-words (16777216), sequence (1000000);
                                      `reduced: true` allows another positive limit (test metadata, never a production constant).
                                      The actual run must carry request(s) with terminal "core" / InputTooLarge, the allowed trace
                                      prefix must match and the required cleanup must be present (see _p6).
  excluded-domain: {name, paths:[...], reached: bool, boundary?}   a named forbidden value; differences only under the paths.
  engine-policy: {id, trigger, fields:[...], realRunner: true}     never applies when policy.commonFake is true.
"""
import sys
sys.dont_write_bytecode = True
import json
import re

CLASSES = ('exact', 'P5', 'P6', 'engine-policy', 'excluded-domain', 'unexplained')
MISSING = {'$missing': 1}
OUTER = ('id', 'requests', 'trace', 'createdSessions', 'openingEvents', 'finalOpening', 'activeTimers', 'activeListeners', 'driver')
PREF = OUTER + ('c', 'r', 'i', 'a', 'o', 'terminal', 'value')
P6_TRIGGERS = {'active-requests': 32, 'packet-words': 16_777_216, 'sequence': 1_000_000}
CLEANUP_PAIRS = (('timer.set', 'timer.clear'), ('listener.add', 'listener.remove'))
CLEANUP_CALLS = ('timer.clear', 'listener.remove', 'rm', 'abort')
MAX_DIFFS = 400


def _order(d):
    pref = [k for k in PREF if k in d]
    return pref + sorted(k for k in d if k not in PREF)


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


class _Refs:
    """First-appearance alpha normalization of opaque reference names, one map for a whole observation."""

    def __init__(self):
        self.map = {}

    def name(self, n):
        if n not in self.map:
            self.map[n] = '@r%d' % len(self.map)
        return self.map[n]


def _normalize(v, refs):
    if isinstance(v, list):
        return [_normalize(x, refs) for x in v]
    if isinstance(v, dict):
        out = {}
        for k in _order(v):
            x = v[k]
            if k == 'ref' and isinstance(x, str):
                out[k] = refs.name(x)
            else:
                out[k] = _normalize(x, refs)
        return out
    return v


def normalize_observation(obs):
    return _normalize(obs, _Refs())


def _diff(e, a, path, out):
    """Appends (path, expected, actual) for every difference in canonical order (stops at MAX_DIFFS)."""
    if len(out) >= MAX_DIFFS:
        return
    te, ta = _tag(e), _tag(a)
    if te != ta:
        out.append((path, e, a))
    elif te == 'dict':
        for k in _order(e):
            if k not in a:
                out.append((path + _key(k), e[k], MISSING))
            else:
                _diff(e[k], a[k], path + _key(k), out)
        for k in _order(a):
            if k not in e:
                out.append((path + _key(k), MISSING, a[k]))
    elif te == 'list':
        for i in range(min(len(e), len(a))):
            _diff(e[i], a[i], '%s[%d]' % (path, i), out)
        for i in range(min(len(e), len(a)), max(len(e), len(a))):
            out.append(('%s[%d]' % (path, i), e[i] if i < len(e) else MISSING, a[i] if i < len(a) else MISSING))
    elif e != a:
        out.append((path, e, a))


def diff_observations(expected, actual):
    """All differences (path, expected, actual) after ref normalization; [] when equal."""
    out = []
    _diff(normalize_observation(expected), normalize_observation(actual), '', out)
    return out


def _result(cls, diff=None):
    if diff is None:
        return {'class': cls, 'path': None, 'expected': None, 'actual': None}
    return {'class': cls, 'path': diff[0].lstrip('.'), 'expected': diff[1], 'actual': diff[2]}


def _pattern(p):
    return re.compile('^' + re.escape(p).replace(r'\[\*\]', r'\[\d+\]') + r'(?:$|[.\[])')


def _covered(path, patterns):
    path = path.lstrip('.')
    return any(_pattern(p).match(path) for p in patterns)


def _is_prose(v):
    return isinstance(v, str) or (isinstance(v, dict) and set(v) == {'u16'})


def _p5(diffs, spec):
    paths = (spec or {}).get('paths') or []
    if not paths:
        return False
    return all(_covered(p, paths) and _is_prose(e) and _is_prose(a) for p, e, a in diffs)


def _excluded(diffs, spec):
    spec = spec or {}
    if not spec.get('name') or not spec.get('paths') or not isinstance(spec.get('reached'), bool):
        return False
    return all(_covered(p, spec['paths']) for p, e, a in diffs)


def _engine(diffs, policy):
    spec = policy.get('engine-policy') or {}
    if policy.get('commonFake') or not spec.get('realRunner'):
        return False
    if not (spec.get('id') and spec.get('trigger') and spec.get('fields')):
        return False
    return all(_covered(p, spec['fields']) for p, e, a in diffs)


def _rejected(actual):
    out = []
    for r in actual.get('requests', []):
        v = r.get('value') if isinstance(r, dict) else None
        if isinstance(r, dict) and r.get('terminal') == 'core' and isinstance(v, dict) and v.get('code') == 'InputTooLarge':
            out.append(r.get('id'))
    return out


def _trigger_ok(spec):
    t = (spec or {}).get('trigger') or {}
    name, limit = t.get('name'), t.get('limit')
    if name not in P6_TRIGGERS or not isinstance(limit, int) or isinstance(limit, bool) or limit < 1:
        return None
    if limit != P6_TRIGGERS[name] and not t.get('reduced'):
        return None
    return name, limit


def _session_ids(req):
    v = req.get('value')
    if isinstance(v, dict) and isinstance(v.get('obj'), list):
        return [x[1] for x in v['obj'] if isinstance(x, list) and len(x) == 2 and x[0] == 'sessionId' and isinstance(x[1], str)]
    return []


def _drop_requests(obs, ids, sessions):
    o = dict(obs)
    o['requests'] = [r for r in obs.get('requests', []) if r.get('id') not in ids]
    o['trace'] = [t for t in obs.get('trace', []) if t.get('r') not in ids]
    o['openingEvents'] = [t for t in obs.get('openingEvents', []) if t.get('r') not in ids]
    o['createdSessions'] = [s for s in obs.get('createdSessions', []) if s.get('id') not in sessions]
    return o


def _cleanup_balanced(trace, pairs):
    for opener, closer in pairs:
        if sum(1 for t in trace if t.get('c') == opener) > sum(1 for t in trace if t.get('c') == closer):
            return '%s without %s' % (opener, closer)
    return None


def _call_matches(t, call):
    """A driver settle names its hold as 'reader:<engine>.<op>' or the bare trace call name."""
    if call.startswith('reader:') and t.get('c') == 'reader' and isinstance(t.get('a'), dict):
        return call == 'reader:%s.%s' % (t['a'].get('engine'), t['a'].get('op'))
    return t.get('c') == call


def _drop_rejected_settles(full, trace, rejset):
    """Drop driver settle lines (r None) whose hold - the k-th call of that name in the oracle trace, label 'x' or
    'x#k' - was created by a rejected request. The implementation never creates that hold."""
    out = []
    for t in trace:
        a = t.get('a') if isinstance(t.get('a'), dict) else {}
        if t.get('c') == 'settle' and t.get('r') is None and isinstance(a.get('call'), str) and isinstance(a.get('label'), str):
            m = re.search(r'#(\d+)$', a['label'])
            k = int(m.group(1)) if m else 1
            owners = [x.get('r') for x in full if x.get('r') is not None and _call_matches(x, a['call'])]
            if 0 < k <= len(owners) and owners[k - 1] in rejset:
                continue
        out.append(t)
    return out


RENUMBER = (('call', None), ('id', 'timer'))


def _renumber(trace):
    """Per-family host counters (a.call of sessionList*, a.id of timer.*) renumbered in order of first appearance.
    Counters consumed by a rejected request leave gaps on the oracle side only; the order and sharing are kept."""
    maps = {}
    out = []
    for t in trace:
        a = t.get('a')
        if isinstance(a, dict) and isinstance(t.get('c'), str):
            fam = t['c'].split('.')[0]
            for key, only in RENUMBER:
                v = a.get(key)
                if (only is None or fam == only) and isinstance(v, int) and not isinstance(v, bool):
                    m = maps.setdefault((fam, key), {})
                    a = dict(a)
                    a[key] = m.setdefault(v, len(m) + 1)
            if a is not t.get('a'):
                t = dict(t, a=a)
        out.append(t)
    return out


def _p6(expected, actual, spec):
    """-> (matched, first diff or None). Trigger + rejected requests + trace prefix + cleanup, see module doc."""
    trig = _trigger_ok(spec)
    rej = _rejected(actual)
    if not trig or not rej:
        return False, None
    name, limit = trig
    exp_ids = [r.get('id') for r in expected.get('requests', [])]
    rejset = set(rej)
    if name == 'active-requests':
        # rejected requests are the ones beyond the limit in start order; the rest must equal the oracle exactly
        if not rejset <= set(exp_ids[limit:]):
            return False, None
        sessions = set()
        for r in expected.get('requests', []):
            if r.get('id') in rejset:
                sessions.update(_session_ids(r))
        exp = _drop_requests(expected, rejset, sessions)
        act = dict(actual)
        act['requests'] = [r for r in actual.get('requests', []) if r.get('id') not in rejset]
        for t in actual.get('trace', []) + actual.get('openingEvents', []):
            if t.get('r') in rejset and t.get('c') not in ('request.start', 'request.done', None):
                return False, (' effect of rejected request %s' % t.get('r'), None, t)
        act['trace'] = [t for t in actual.get('trace', []) if t.get('r') not in rejset]
        act['openingEvents'] = [t for t in actual.get('openingEvents', []) if t.get('r') not in rejset]
        if actual.get('driver') is not None:
            return False, None
        exp['trace'] = _drop_rejected_settles(expected.get('trace', []), exp['trace'], rejset)
        exp['trace'], act['trace'] = _renumber(exp['trace']), _renumber(act['trace'])
        d = diff_observations(exp, act)
        if d:
            return False, d[0]
        return True, None
    # packet-words / sequence: honest prefix, then cleanup, then the failure
    strip = lambda tr: [t for t in tr if t.get('c') not in CLEANUP_CALLS]
    et, at = strip(expected.get('trace', [])), strip(actual.get('trace', []))
    if len(at) > len(et):
        return False, None
    pre = diff_observations(et[:len(at)], at)
    if pre:
        return False, pre[0]
    pairs = [tuple(p) for p in spec.get('requiredCleanup', CLEANUP_PAIRS)]
    bad = _cleanup_balanced(actual.get('trace', []), pairs)
    if bad:
        return False, ('trace (cleanup: %s)' % bad, None, None)
    if actual.get('activeTimers') or any(actual.get('activeListeners', {}).values()):
        return False, ('activeTimers/activeListeners (cleanup missing)', None, None)
    by_exp = {r.get('id'): r for r in expected.get('requests', [])}
    for r in actual.get('requests', []):
        if r.get('id') in rejset:
            continue
        if r != by_exp.get(r.get('id')):
            return False, ('requests (%s)' % r.get('id'), by_exp.get(r.get('id')), r)
    return True, None


def compare_observation(expected, actual, policy=None):
    """-> {class, path, expected, actual}. path/expected/actual describe the first difference (None when exact).
    Missing keys are shown as {"$missing":1}."""
    policy = policy or {}
    diffs = diff_observations(expected, actual)
    p6 = policy.get('P6')
    if not diffs:
        if p6 and p6.get('mustTrigger'):
            return {'class': 'unexplained', 'path': 'requests', 'expected': 'P6 trigger %s' % json.dumps(p6.get('trigger')),
                    'actual': 'bound not enforced (identical to the unbounded oracle)'}
        return _result('exact')
    first = diffs[0]
    if policy.get('P5') and _p5(diffs, policy['P5']):
        return _result('P5', first)
    if policy.get('excluded-domain') and _excluded(diffs, policy['excluded-domain']):
        return _result('excluded-domain', first)
    if policy.get('engine-policy') and _engine(diffs, policy):
        return _result('engine-policy', first)
    if p6:
        ok, d = _p6(expected, actual, p6)
        if ok:
            return _result('P6', first)
        if d is not None:
            return _result('unexplained', d)
    return _result('unexplained', first)


# ---- fixtures policy and batches ----
def default_policy(fixture_id):
    """P08 is an honest 33-open Node scenario; the implementation must reject beyond 32 active requests (P6)."""
    if fixture_id == 'P08':
        return {'P6': {'trigger': {'name': 'active-requests', 'limit': 32}, 'mustTrigger': True}}
    return None


def _has_invariant(v):
    if isinstance(v, dict):
        return v.get('code') == 'InternalInvariant' or any(_has_invariant(x) for x in v.values())
    if isinstance(v, list):
        return any(_has_invariant(x) for x in v)
    return False


def _fail(id_, reason, path, expected=None, actual=None):
    return {'id': id_, 'class': 'unexplained', 'path': path, 'expected': expected, 'actual': actual, 'reason': reason}


def compare_batch(expected, actual, policies=None):
    """One result per expected scenario, in order. `actual`: list of observation dicts, JSON lines (str), exceptions or None
    (crash / missing). `policies`: dict id -> policy or callable(id, expected_obs) -> policy (default: default_policy).
    Returns {results, counts, failures, ok}; every failure has class unexplained and a `reason`:
    crash, missing, malformed, id-mismatch, driver, internal-invariant, extra."""
    results = []
    if isinstance(actual, BaseException) or (isinstance(actual, dict) and 'crash' in actual):
        actual = []
        crash = True
    else:
        actual, crash = list(actual or []), False
    for i, exp in enumerate(expected):
        eid = exp.get('id')
        if i >= len(actual):
            results.append(_fail(eid, 'crash' if crash else 'missing', 'id', eid, None))
            continue
        act = actual[i]
        if isinstance(act, BaseException) or (isinstance(act, dict) and 'crash' in act):
            results.append(_fail(eid, 'crash', 'id', eid, str(act)[:300]))
            continue
        if isinstance(act, (str, bytes)):
            try:
                act = json.loads(act)
            except ValueError:
                results.append(_fail(eid, 'malformed', 'json', eid, str(act)[:200]))
                continue
        if not isinstance(act, dict) or 'requests' not in act or 'trace' not in act:
            results.append(_fail(eid, 'malformed', 'shape', eid, str(act)[:200]))
            continue
        if act.get('id') != eid:
            results.append(_fail(eid, 'id-mismatch', 'id', eid, act.get('id')))
            continue
        if policies is None:
            pol = default_policy(eid)
        elif callable(policies):
            pol = policies(eid, exp)
        else:
            pol = policies.get(eid)
        if _has_invariant(act):
            results.append(_fail(eid, 'internal-invariant', 'requests', None, 'InternalInvariant'))
            continue
        if act.get('driver') is not None and not (pol or {}).get('expectDriver'):
            results.append(_fail(eid, 'driver', 'driver', exp.get('driver'), act.get('driver')))
            continue
        r = compare_observation(exp, act, pol)
        r['id'] = eid
        results.append(r)
    for extra in actual[len(expected):]:
        results.append(_fail(extra.get('id') if isinstance(extra, dict) else None, 'extra', 'id', None, str(extra)[:200]))
    counts = {c: 0 for c in CLASSES}
    for r in results:
        counts[r['class']] += 1
    return {'results': results, 'counts': counts, 'failures': counts['unexplained'], 'ok': counts['unexplained'] == 0}
