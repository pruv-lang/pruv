"""Runs every fixture (fixtures/<group>.json, 160) through the Node oracle, checks each fixture's explicit expectations and reports per-group
pass counts. Also runs the whole batch twice and requires identical output (determinism). Exit code 0 only if all 160 pass.
  python3 check_oracle.py [-v] [--group S] [--id S05]
Expectation language: SCENARIO.md section 8."""
import sys
sys.dont_write_bytecode = True
import json

from common import GROUPS, load_all, load_group
from oracle import run_oracle


def plain(jv):
    """JV -> python. undefined -> '@undef', opaque ref -> '@ref:name', cancel marker untouched (only inside thrown values)."""
    if isinstance(jv, dict):
        if 'u16' in jv and len(jv) == 1:
            return ''.join(chr(u) for u in jv['u16'])
        if 'undef' in jv and len(jv) == 1:
            return '@undef'
        if 'ref' in jv and len(jv) == 1:
            return '@ref:' + str(jv['ref'])
        if 'num' in jv and len(jv) == 1:
            return {'@num': jv['num']}
        if 'arr' in jv and len(jv) == 1:
            return [plain(x) for x in jv['arr']]
        if 'obj' in jv and len(jv) == 1:
            return {(k if isinstance(k, str) else plain(k)): plain(v) for k, v in jv['obj']}
        return {k: plain(v) for k, v in jv.items()}
    if isinstance(jv, list):
        return [plain(x) for x in jv]
    return jv


def sub(expected, actual, path='$'):
    """Subset match: dict keys listed must match, lists match element-wise with equal length, scalars equal. Returns a message or None."""
    if isinstance(expected, dict):
        if not isinstance(actual, dict):
            return f'{path}: expected object, got {actual!r}'
        for k, v in expected.items():
            if k not in actual:
                return f'{path}.{k}: missing (have {list(actual)})'
            m = sub(v, actual[k], f'{path}.{k}')
            if m:
                return m
        return None
    if isinstance(expected, list):
        if not isinstance(actual, list) or len(actual) != len(expected):
            return f'{path}: expected list of {len(expected)}, got {actual!r}'
        for i, (e, a) in enumerate(zip(expected, actual)):
            m = sub(e, a, f'{path}[{i}]')
            if m:
                return m
        return None
    if expected != actual or type(expected) != type(actual):
        return f'{path}: expected {expected!r}, got {actual!r}'
    return None


def rec_matches(spec, rec):
    """spec: {c, r?, i?, a?, o?} subset-matched against a trace record (a and o are compared as plain values)."""
    for key in ('c', 'r', 'i'):
        if key in spec and spec[key] != rec.get(key):
            return False
    for key in ('a', 'o'):
        if key in spec and sub(spec[key], plain(rec.get(key))) is not None:
            return False
    return True


def find(trace, spec, start=0):
    for i in range(start, len(trace)):
        if rec_matches(spec, trace[i]):
            return i
    return -1


def as_spec(x):
    return {'c': x} if isinstance(x, str) else x


def check(fx, obs):
    errs = []
    ex = fx.get('expect', {})
    if obs.get('driver') is not None:
        errs.append(f"driver violations: {obs['driver']}")
    requests = {r['id']: r for r in obs['requests']}
    trace = obs['trace']
    for rid, want in ex.get('req', {}).items():
        got = requests.get(rid)
        if got is None:
            errs.append(f'request {rid} missing')
            continue
        if 'terminal' in want and got['terminal'] != want['terminal']:
            errs.append(f"{rid}: terminal {got['terminal']} != {want['terminal']} ({json.dumps(got['value'], ensure_ascii=True)[:300]})")
            continue
        value = plain(got['value'])
        if got['terminal'] == 'wire':
            wv = value['wire']
            if 'key' in want and plain(wv['key']) != want['key'] and plain(wv['key']) != {'key': want['key']}:
                errs.append(f"{rid}: wire key {wv['key']!r} != {want['key']!r}")
            if 'message' in want and wv['message'] != want['message']:
                errs.append(f"{rid}: wire message {wv['message']!r} != {want['message']!r}")
        if got['terminal'] == 'thrown':
            got_ref = value.get('ref') if isinstance(value, dict) else (value[5:] if isinstance(value, str) and value.startswith('@ref:') else None)
            if 'ref' in want and got_ref != want['ref']:
                errs.append(f"{rid}: thrown {value!r} != ref {want['ref']!r}")
            cr = value.get('cancelReason') if isinstance(value, dict) else None
            if 'reason' in want and cr != want['reason']:
                errs.append(f"{rid}: cancelReason {cr!r} != {want['reason']!r}")
            if 'noReason' in want and cr is not None:
                errs.append(f'{rid}: unexpected cancelReason')
        if got['terminal'] == 'eie' and 'code' in want and value['eie']['code'] != want['code']:
            errs.append(f"{rid}: eie code {value['eie']['code']!r} != {want['code']!r}")
        if 'value' in want:
            m = sub(want['value'], value)
            if m:
                errs.append(f'{rid}: value {m}')
        if 'itemKeys' in want:
            items = value['items']
            for idx, ks in enumerate(want['itemKeys']):
                if ks is not None and list(items[idx].keys()) != ks:
                    errs.append(f"{rid}: item[{idx}] keys {list(items[idx].keys())} != {ks}")
        if 'itemCount' in want and len(value['items']) != want['itemCount']:
            errs.append(f"{rid}: itemCount {len(value['items'])} != {want['itemCount']}")
        if 'keys' in want:
            keys = list(value.keys()) if isinstance(value, dict) else None
            if keys != want['keys']:
                errs.append(f"{rid}: keys {keys} != {want['keys']}")
        for text in want.get('notContains', []):
            if text in json.dumps(got['value'], ensure_ascii=True):
                errs.append(f'{rid}: value contains {text!r}')
    for c, n in ex.get('counts', {}).items():
        have = sum(1 for r in trace if r['c'] == c)
        if have != n:
            errs.append(f'count {c}: {have} != {n}')
    for spec in ex.get('has', []):
        if find(trace, as_spec(spec)) < 0:
            errs.append(f'trace lacks {spec}')
    for spec in ex.get('absent', []):
        if find(trace, as_spec(spec)) >= 0:
            errs.append(f'trace has forbidden {spec}')
    if 'order' in ex:
        pos = 0
        for spec in ex['order']:
            i = find(trace, as_spec(spec), pos)
            if i < 0:
                errs.append(f'order: {spec} not found after position {pos}')
                break
            pos = i + 1
    for first, second, banned in ex.get('noneBetween', []):
        i = find(trace, as_spec(first))
        j = find(trace, as_spec(second), i + 1) if i >= 0 else -1
        if i < 0 or j < 0:
            errs.append(f'noneBetween: anchors not found {first} {second}')
            continue
        if find(trace[:j], as_spec(banned), i + 1) >= 0:
            errs.append(f'noneBetween: {banned} occurs between {first} and {second}')
    if 'created' in ex and len(obs['createdSessions']) != ex['created']:
        errs.append(f"created {len(obs['createdSessions'])} != {ex['created']}")
    for i, want in enumerate(ex.get('createdAfter', [])):
        if want is None:
            continue
        m = sub(want, plain(obs['createdSessions'][i]['after'])) if i < len(obs['createdSessions']) else 'missing'
        if m:
            errs.append(f'createdAfter[{i}] {m}')
    for i, want in enumerate(ex.get('createdBefore', [])):
        if want is not None:
            m = sub(want, plain(obs['createdSessions'][i]['before'])) if i < len(obs['createdSessions']) else 'missing'
            if m:
                errs.append(f'createdBefore[{i}] {m}')
    for i, want in enumerate(ex.get('createdMeta', [])):
        if want is not None:
            m = sub(want, {k: plain(v) for k, v in obs['createdSessions'][i].items() if k not in ('before', 'after')}) if i < len(obs['createdSessions']) else 'missing'
            if m:
                errs.append(f'createdMeta[{i}] {m}')
    for i, path, want in ex.get('createdKeys', []):
        node = plain(obs['createdSessions'][i]['after'])
        for p in path:
            node = node[p]
        if list(node.keys()) != want:
            errs.append(f'createdKeys[{i}] {path}: {list(node.keys())} != {want}')
    for i, path, n in ex.get('createdLen', []):
        node = plain(obs['createdSessions'][i]['after'])
        for p in path:
            node = node[p]
        if len(node) != n:
            errs.append(f'createdLen[{i}] {path}: {len(node)} != {n}')
    for spec, n in ex.get('matchCounts', []):
        have = sum(1 for r in trace if rec_matches(as_spec(spec), r))
        if have != n:
            errs.append(f'matchCount {spec}: {have} != {n}')
    if 'opening' in ex:
        got = {k: [plain(x) for x in v] for k, v in obs['finalOpening'].items()}
        if got != ex['opening']:
            errs.append(f"finalOpening {got} != {ex['opening']}")
    if 'openingOps' in ex:
        got = [f"{e['op']}:{plain(e['key'])}:{str(e['result']).lower()}" + (f"@{e['r']}" if e.get('r') and ex.get('openingWithReq') else '') for e in obs['openingEvents']]
        if got != ex['openingOps']:
            errs.append(f"openingOps {got} != {ex['openingOps']}")
    if 'timers' in ex and len(obs['activeTimers']) != ex['timers']:
        errs.append(f"activeTimers {obs['activeTimers']} != {ex['timers']}")
    if 'listeners' in ex and obs['activeListeners'] != ex['listeners']:
        errs.append(f"activeListeners {obs['activeListeners']} != {ex['listeners']}")
    return errs


def main(argv):
    verbose = '-v' in argv
    only_group = argv[argv.index('--group') + 1] if '--group' in argv else None
    only_id = argv[argv.index('--id') + 1] if '--id' in argv else None
    pool = load_group(only_group) if only_group else load_all()
    fixtures = [f for f in pool if (only_group is None or f['id'][0] == only_group) and (only_id is None or f['id'] == only_id)]
    first = run_oracle(fixtures)
    second = run_oracle(fixtures)
    deterministic = first == second
    per = {g: [0, 0] for g in GROUPS}
    failures = []
    for fx, obs in zip(fixtures, first):
        errs = check(fx, obs)
        per[fx['id'][0]][1] += 1
        if not errs:
            per[fx['id'][0]][0] += 1
        else:
            failures.append((fx['id'], fx.get('name'), errs))
        if verbose:
            print(fx['id'], 'ok' if not errs else 'FAIL')
    for gid, name, errs in failures:
        print(f'FAIL {gid} {name}')
        for e in errs[:8]:
            print('   ', e)
    total_ok = sum(v[0] for v in per.values())
    total = sum(v[1] for v in per.values())
    print('per group: ' + ' '.join(f'{g}={v[0]}/{v[1]}' for g, v in per.items() if v[1]))
    print(f"determinism: {'identical' if deterministic else 'DIFFERENT'}")
    print(f"oracle check: {total_ok}/{total} fixtures pass" + ('' if deterministic else ' (NONDETERMINISTIC)'))
    return 0 if (total_ok == total and deterministic and (only_group or only_id or total == 160)) else 1


if __name__ == '__main__':
    sys.exit(main(sys.argv))
