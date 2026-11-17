"""L3-10 oracle fixture check: runs every fixed fixture (fixtures/<G>.json) through the Node oracle twice (determinism: the two
JSON outputs must be identical) and checks each fixture's `expect` with the DSL below. Per-group pass/fail is printed; exit 0 only
when every fixture passes, the runs are identical and no `expect` has a checker error.
  python3 check_oracle.py [--group A] [--verbose]
Failure details: work/check/fail/<id>.json

`expect` is a dict, every key optional, at least one required; an unknown key is a CHECKER ERROR (reported, never a pass):
  requests  [{req, terminal?, value?, valueKeys?}]   value: subset match on the decoded value (dicts: listed keys must match,
                                                       recursively; lists and scalars exactly, type strict). valueKeys: the exact
                                                       ordered key list of the decoded dict value.
  counts    {c: n}              exact number of trace records with that c
  has       [c, ...]            each c occurs in the trace
  absent    [c, ...]            no c occurs in the trace
  order     [[c1, c2], ...]     first c1 occurs before first c2 (both must occur)
  session   [{id, path, value} | {id, path, absent:true} | {id, path, keys:[...]}]   decoded finalSessions: the session whose
                                         decoded id matches, then the dotted path (numeric parts index arrays; a key list is
                                         also accepted); value subset-matched, keys = exact ordered key list
  clocks    {count?, has?, absent?}      finalClocks over every instance: entry count, budget ids present / absent
  timers    n                   number of active timers
  driver    null | {...}        default (key absent): the driver must be null; null = same; a dict is subset matched
Decoding: compare.jv_decode, with {sref, v} / {bref, v} snapshot wrappers replaced by their v."""
import sys
sys.dont_write_bytecode = True
import json
import math
from collections import OrderedDict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from common import GROUPS, HERE, load_all, load_group  # noqa: E402
from compare import UNDEF, jv_decode  # noqa: E402
from oracle import run_oracle  # noqa: E402

FAIL_DIR = HERE / 'work' / 'check' / 'fail'
KEYS = ('requests', 'counts', 'has', 'absent', 'order', 'session', 'clocks', 'timers', 'driver')
REQ_KEYS = ('req', 'terminal', 'value', 'valueKeys')


def unwrap(x):
    """Replace {sref, v} / {bref, v} snapshot wrappers by their decoded v."""
    if isinstance(x, dict):
        if set(x) == {'sref', 'v'} or set(x) == {'bref', 'v'}:
            return unwrap(x['v'])
        return {k: unwrap(v) for k, v in x.items()}
    if isinstance(x, list):
        return [unwrap(v) for v in x]
    return x


def dec(jv):
    return unwrap(jv_decode(jv))


def _tag(v):
    if v is UNDEF:
        return 'undef'
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


def sub(expected, actual, path='$'):
    """Subset match (message or None). dict: listed keys must match recursively; list: equal length, element-wise subset-free exact
    match (spec: lists exact); scalars: equal and same type (NaN equals NaN)."""
    te, ta = _tag(expected), _tag(actual)
    if te == 'dict':
        if ta != 'dict':
            return f'{path}: expected object, got {actual!r}'
        for k, v in expected.items():
            if k not in actual:
                return f'{path}.{k}: missing (have {list(actual)})'
            m = sub(v, actual[k], f'{path}.{k}')
            if m:
                return m
        return None
    if te == 'list':
        if ta != 'list' or len(actual) != len(expected):
            return f'{path}: expected list of {len(expected)}, got {actual!r}'
        for i, (e, a) in enumerate(zip(expected, actual)):
            m = exact(e, a, f'{path}[{i}]')
            if m:
                return m
        return None
    return exact(expected, actual, path)


def exact(expected, actual, path='$'):
    te, ta = _tag(expected), _tag(actual)
    if te != ta:
        return f'{path}: expected {expected!r}, got {actual!r}'
    if te == 'dict':
        if set(expected) != set(actual):
            return f'{path}: keys {sorted(actual)} != {sorted(expected)}'
        for k in expected:
            m = exact(expected[k], actual[k], f'{path}.{k}')
            if m:
                return m
        return None
    if te == 'list':
        if len(expected) != len(actual):
            return f'{path}: length {len(actual)} != {len(expected)}'
        for i, (e, a) in enumerate(zip(expected, actual)):
            m = exact(e, a, f'{path}[{i}]')
            if m:
                return m
        return None
    if te == 'float' and math.isnan(expected) and math.isnan(actual):
        return None
    if expected != actual:
        return f'{path}: expected {expected!r}, got {actual!r}'
    return None


def check(fx, obs):
    """-> (errors, checker_errors) for one fixture."""
    errs, bad = [], []
    ex = fx.get('expect')
    if not isinstance(ex, dict) or not ex:
        return errs, ['expect must be a non-empty dict']
    for k in ex:
        if k not in KEYS:
            bad.append(f'unknown expect key {k!r}')
    if bad:
        return errs, bad
    requests = obs.get('requests', [])
    trace = obs.get('trace', [])
    for r in requests:
        if r.get('terminal') not in ('ok', 'thrown'):
            errs.append(f"request {r.get('id')}: terminal {r.get('terminal')!r} (only ok/thrown are allowed)")
    if 'driver' in ex:
        want = ex['driver']
        if want is None:
            if obs.get('driver') is not None:
                errs.append(f"driver {obs['driver']} != null")
        else:
            m = sub(want, obs.get('driver'), 'driver')
            if m:
                errs.append(m)
    elif obs.get('driver') is not None:
        errs.append(f"driver violations: {obs['driver']}")
    by_id = {r['id']: r for r in requests}
    for spec in ex.get('requests', []):
        if not isinstance(spec, dict) or 'req' not in spec or any(k not in REQ_KEYS for k in spec):
            bad.append(f'bad requests entry {spec!r} (keys: {", ".join(REQ_KEYS)})')
            continue
        got = by_id.get(spec['req'])
        if got is None:
            errs.append(f"request {spec['req']} missing")
            continue
        if 'terminal' in spec and got['terminal'] != spec['terminal']:
            errs.append(f"{spec['req']}: terminal {got['terminal']} != {spec['terminal']} ({json.dumps(got['value'], ensure_ascii=True)[:300]})")
            continue
        value = dec(got['value'])
        if 'value' in spec:
            m = sub(dec(spec['value']), value, f"{spec['req']}.value")
            if m:
                errs.append(m)
        if 'valueKeys' in spec:
            if not isinstance(value, dict):
                errs.append(f"{spec['req']}: valueKeys on a non-object value {value!r}")
            else:
                if list(value) != list(spec['valueKeys']):
                    errs.append(f"{spec['req']}: value keys {list(value)} != {spec['valueKeys']}")
    counts = {}
    first = {}
    for i, t in enumerate(trace):
        counts[t['c']] = counts.get(t['c'], 0) + 1
        first.setdefault(t['c'], i)
    for c, n in ex.get('counts', {}).items():
        if counts.get(c, 0) != n:
            errs.append(f'count {c}: {counts.get(c, 0)} != {n}')
    for c in ex.get('has', []):
        if c not in counts:
            errs.append(f'trace lacks {c}')
    for c in ex.get('absent', []):
        if c in counts:
            errs.append(f'trace has forbidden {c}')
    for pair in ex.get('order', []):
        if not (isinstance(pair, list) and len(pair) == 2):
            bad.append(f'bad order entry {pair!r}')
            continue
        a, b = pair
        if a not in first or b not in first:
            errs.append(f'order: {a if a not in first else b} not in trace')
        elif first[a] >= first[b]:
            errs.append(f'order: first {a} (#{first[a]}) is not before first {b} (#{first[b]})')
    if 'session' in ex:
        sessions = [dec(s) for s in obs.get('finalSessions', [])]
        for spec in ex['session']:
            kinds = [k for k in ('value', 'absent', 'keys') if k in spec] if isinstance(spec, dict) else []
            if not isinstance(spec, dict) or 'id' not in spec or 'path' not in spec or len(kinds) != 1 \
                    or ('absent' in spec and spec['absent'] is not True):
                bad.append(f'bad session entry {spec!r}')
                continue
            sess = next((s for s in sessions if isinstance(s, dict) and s.get('id') == spec['id']), None)
            if sess is None:
                errs.append(f"session {spec['id']!r} not in finalSessions")
                continue
            # SCENARIO.md: a dotted path string (numeric parts index arrays); a key list is also accepted.
            path = spec['path'].split('.') if isinstance(spec['path'], str) else list(spec['path'])
            if path == ['']:
                path = []
            node, missing = sess, None
            for key in path:
                if isinstance(node, dict) and key in node:
                    node = node[key]
                elif isinstance(node, list) and str(key).isdigit() and int(key) < len(node):
                    node = node[int(key)]
                else:
                    missing = key
                    break
            where = f"session[{spec['id']}].{'.'.join(map(str, path))}"
            if 'absent' in spec:
                if missing is None:
                    errs.append(f'{where}: present ({node!r}), expected absent')
                continue
            if missing is not None:
                errs.append(f'{where}: path breaks at {missing!r}')
                continue
            if 'keys' in spec:
                got = list(node) if isinstance(node, dict) else None
                if got != list(spec['keys']):
                    errs.append(f"{where}: keys {got} != {spec['keys']}")
                continue
            m = sub(dec(spec['value']), node, where)
            if m:
                errs.append(m)
    if 'clocks' in ex:
        # SCENARIO.md: {count?, has?, absent?} over the clocks of every instance (budget ids).
        spec = ex['clocks']
        if not isinstance(spec, dict) or any(k not in ('count', 'has', 'absent') for k in spec):
            bad.append(f'bad clocks entry {spec!r}')
        else:
            ids = []
            for entries in obs.get('finalClocks', {}).values():
                for entry in entries:
                    ids.append(dec(entry[0]) if isinstance(entry, list) else dec(entry))
            if 'count' in spec and len(ids) != spec['count']:
                errs.append(f"clocks count {len(ids)} != {spec['count']}")
            for b in spec.get('has', []):
                if b not in ids:
                    errs.append(f'clocks lack {b}')
            for b in spec.get('absent', []):
                if b in ids:
                    errs.append(f'clocks have forbidden {b}')
    if 'timers' in ex and len(obs.get('activeTimers', [])) != ex['timers']:
        errs.append(f"activeTimers {len(obs.get('activeTimers', []))} != {ex['timers']}")
    return errs, bad


def main(argv):
    verbose = '--verbose' in argv or '-v' in argv
    group = argv[argv.index('--group') + 1].upper() if '--group' in argv else None
    if group is not None and group not in GROUPS:
        print(f'unknown group {group!r} (use one of {GROUPS})')
        return 2
    fixtures = load_group(group) if group else load_all()
    if not fixtures:
        print('no fixtures found')
        return 1
    first = run_oracle(fixtures)
    second = run_oracle(fixtures)
    deterministic = json.dumps(first, sort_keys=True, ensure_ascii=True) == json.dumps(second, sort_keys=True, ensure_ascii=True)
    per = OrderedDict((g, [0, 0]) for g in GROUPS)
    FAIL_DIR.mkdir(parents=True, exist_ok=True)
    for fx in fixtures:
        old = FAIL_DIR / f"{fx['id']}.json"
        if old.exists():
            old.unlink()
    failed, checker_errors = [], []
    for fx, obs in zip(fixtures, first):
        errs, bad = check(fx, obs)
        g = fx['id'][0]
        per.setdefault(g, [0, 0])[1] += 1
        if not errs and not bad:
            per[g][0] += 1
        else:
            (checker_errors if bad else failed).append((fx, errs, bad))
            (FAIL_DIR / f"{fx['id']}.json").write_text(json.dumps(
                {'id': fx['id'], 'name': fx.get('name'), 'rule': fx.get('rule'), 'expect': fx.get('expect'), 'errors': errs,
                 'checkerErrors': bad, 'observation': obs}, ensure_ascii=False, indent=1), encoding='utf-8')
        if verbose:
            print(fx['id'], 'ok' if not errs and not bad else ('CHECKER-ERROR' if bad else 'FAIL'))
    for fx, errs, bad in failed + checker_errors:
        print(f"{'CHECKER ERROR' if bad else 'FAIL'} {fx['id']} {fx.get('name', '')}")
        for e in (bad + errs)[:8]:
            print('   ', e)
    for g, (ok, total) in per.items():
        if total:
            print(f'group {g}: {ok}/{total} pass' + ('' if ok == total else f' ({total - ok} fail)'))
    total_ok = sum(v[0] for v in per.values())
    total = sum(v[1] for v in per.values())
    print(f"determinism: {'identical' if deterministic else 'DIFFERENT'}")
    if group is None and total != 140:
        print(f'note: {total} fixtures loaded (the spec plans 140)')
    print(f"oracle check: {total_ok}/{total} fixtures pass, {len(checker_errors)} checker errors" + ('' if deterministic else ' (NONDETERMINISTIC)'))
    return 0 if total_ok == total and deterministic and not checker_errors else 1


if __name__ == '__main__':
    sys.exit(main(sys.argv))
