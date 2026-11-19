#!/usr/bin/env python3
"""Replay M12 against the pinned oracle, check mutants and hand parity, or time both lanes."""
import sys
sys.dont_write_bytecode = True

import argparse
from collections import Counter, defaultdict
import os
from pathlib import Path
import shutil
import statistics
import subprocess

HERE = Path(__file__).resolve().parent
BUDGET = HERE.parent
REPO = HERE.parents[3]
WORK = REPO / 'work' / 'm12'
sys.path[:0] = [str(BUDGET), str(BUDGET / 'm11_test')]

import common
import oracle
import q_gen
from compare import UNDEF, jv_decode, jv_encode
from run_m11 import (MODULES, PAIRS, PY_DEFAULT, RS_DEFAULT, QUERY, COUNTED, V, canon,
                     from_v, parse_dump, parse_ok, pkey, sid_of, strip_brefs, to_b,
                     tool_env, wline)

AUTO = {'clearAutoContinue': 1, 'recordAutoContinue': 2, 'stopAutoContinue': 3}
SUPPORTED = {*AUTO, *QUERY, 'setCard'}


def run(cmd, *, data=None, env=None, cwd=BUDGET):
    env = dict(tool_env() if env is None else env)
    env.update(TMPDIR=str(WORK / 'tmp'), PYTHONDONTWRITEBYTECODE='1')
    p = subprocess.run([str(x) for x in cmd], cwd=cwd, env=env, input=data,
                       capture_output=True, text=True)
    if p.returncode:
        raise RuntimeError(f'{cmd[0]} exited {p.returncode}\n{p.stdout[-4000:]}\n{p.stderr[-4000:]}')
    return p.stdout


def build(pair, variant='base', source=None):
    out = WORK / variant / pair
    out.mkdir(parents=True, exist_ok=True)
    root = out / 'root.pruv'
    shutil.copy(HERE / 'root.pruv', root)
    for name in MODULES:
        shutil.copy(BUDGET / name, out / name)
    auto = BUDGET / 'bud_auto.pruv'
    (out / auto.name).write_text(auto.read_text() if source is None else source)
    sol = out / 'sol.rs'
    lib = REPO / 'bench' / 'l3' / 'lib'
    if pair == 'py':
        cmd = [os.environ.get('PRUVC_PY_PYTHON', PY_DEFAULT), REPO / 'pruvc' / 'pruvc.py',
               root, '-o', sol, '--backend', 'rust', '--lib', lib]
    else:
        cmd = [os.environ.get('PRUVC_RS', RS_DEFAULT), root, '-o', sol, '--lib', lib]
    run(cmd, cwd=REPO)
    main = out / 'main.rs'
    main.write_text((HERE / 'harness.rs').read_text()
                    .replace('@SOL@', str(sol)).replace('@HAND@', str(HERE / 'hand.rs')))
    run(['rustc', '-O', '--edition', '2021', '-A', 'warnings', main, '-o', out / 'harness'])
    return out / 'harness'


def q7p(seed, count):
    cases = []
    for n in range(1, count + 1):
        r = q_gen.case_rng(seed, 7, n)
        c = q_gen.begin(r, nsess=r.between(2, 3))
        c.snapshot_world()

        def clear():
            c.call('clearAutoContinue', c.refjv(c.pick_ref()))

        def record():
            c.call('recordAutoContinue', c.refjv(c.pick_ref()), q_gen.ac_record(r))

        def stop():
            sid = r.pick([s.sidjv for s in c.live()] + ['nobody'])
            k = r.below(5)
            if k <= 1:
                c.call('stopAutoContinue', sid)
            else:
                c.call('stopAutoContinue', sid, q_gen.U if k == 4 else c.refjv(c.pick_ref()))

        c.run_until([(14, clear), (24, record), (28, stop), (14, lambda: q_gen.e_set_card(c)),
                     (12, c.e_current), (8, c.e_advance)], 5, r.between(8, 18))
        cases.append({'id': f'Q7P-{seed}-{n}', 'world': c.world_final(), 'script': c.steps[:20]})
    return cases


def crafted(big=False):
    sid = '세션😀'
    ref = {'sessionId': sid, 'budgetId': 'b😀'}
    record = {'sessionId': sid, 'stopped': False, 'userTurnId': '😀', 'count': 99}
    card = {'state': 'working', 'by': {'sessionId': sid}, 'deliverable': '한😀' * (5000 if big else 1)}
    out = []
    values = (None, False, 0, '', 'truthy😀', record, {**record, 'stopped': 0}, {**record, 'stopped': 'yes'})
    for i, value in enumerate(values):
        budget = {'id': 'b😀', 'state': 'open', 'autoContinue': value, 'card': card}
        calls = [
            ('clearAutoContinue', ref),
            ('hasCard', ref),
            ('stopAutoContinue', sid, UNDEF),
            ('current', ref),
            ('recordAutoContinue', ref, 'truthy😀'),
            ('stopAutoContinue', sid, ref),
            ('current', ref),
            ('recordAutoContinue', ref, {**record, 'payload': 'é😀' * (8000 if big else 1)}),
            ('stopAutoContinue', sid, UNDEF),
            ('current', ref),
            ('stopAutoContinue', sid),
            ('clearAutoContinue', ref),
            ('recordAutoContinue', ref, value if i >= 5 else record),
            ('stopAutoContinue', sid),
            ('current', ref),
        ]
        out.append({'id': f'crafted-{big}-{i}', 'world': {'sessions': [jv_encode({'id': sid, 'budget': budget})]},
                    'script': [{'ev': 'call', 'req': f'r{j}', 'op': op, 'args': [jv_encode(x) for x in a]}
                               for j, (op, *a) in enumerate(calls, 1)]})
    return out


def fixed_checks(harness):
    """Direct checks the Node oracle cannot express. Returns a list of problems.

    dup: a loaded V object that repeats autoContinue (JSON.parse would collapse it). clear and a falsy record must
    remove every pair and the count, in both lanes. room: an edit whose 1.5x reserve does not fit the free arena
    while the exact size does must not Grow (Pruv lane; the probe reads the host's Grow count)."""
    problems = []
    sid, ref = 's1', {'sessionId': 's1', 'budgetId': 'b1'}
    base = {'id': 'b1', 'state': 'open'}
    head = V({**base, 'autoContinue': 'old'})
    dup = [9, 4] + head[2:] + V({'autoContinue': 'new'})[2:]
    for lane in ([harness], [harness, '--hand']):
        for op in (1, 2):
            lines = ['init', 'load ' + wline(pkey(sid), dup),
                     'auto ' + wline([op], V(ref), sid_of(ref), [], []), 'dump']
            raw = run(lane, data='\n'.join(lines) + '\n').splitlines()
            blobs = dict(parse_dump(parse_ok(raw[-1])))
            if raw[2][:2] != 'ok' or blobs.get(sid) != V(base):
                problems.append(f'dup op {op} {lane[1:]}: {raw[2][:60]} left {blobs.get(sid)}')
    lines, worlds = [], 0
    for pad in range(0, 4000, 7):
        lines += ['init', 'load ' + wline(pkey('n1'), V({'id': 'x', 'pad': 'a' * pad})),
                  'load ' + wline(pkey(sid), V(base)), 'room',
                  'auto ' + wline([2], V(ref), sid_of(ref), [], to_b('v' * 20)), 'dump', 'room']
        worlds += 1
    raw = run([harness], data='\n'.join(lines) + '\n').splitlines()
    tight = 0
    for w in range(worlds):
        r = raw[7 * w:7 * w + 7]
        g0, f0 = parse_ok(r[3])
        g1, _ = parse_ok(r[6])
        m = len(dict(parse_dump(parse_ok(r[5])))[sid])
        tight += m <= f0 < m + m // 2
        if g1 != g0 and m <= f0:
            problems.append(f'room pad world {w}: Grow with m={m} free={f0}')
    if not tight:
        problems.append('room sweep never hit the reserve-does-not-fit, size-fits window')
    print(f'fixed checks: dup 4 runs, room {worlds} worlds ({tight} in the tight window), {len(problems)} problems', flush=True)
    return problems


def cut_case(case):
    for i, event in enumerate(case['script']):
        if event['ev'] not in ('call', 'advance'):
            return {**case, 'script': case['script'][:i]}, True
        if event['ev'] == 'call' and (event['op'] not in SUPPORTED or event.get('i', 'A') != 'A'):
            return {**case, 'script': case['script'][:i]}, True
    return case, False


def case_lines(case):
    lines = ['init']
    for raw in case['world'].get('sessions', []):
        session = jv_decode(raw)
        budget = session.get('budget', UNDEF)
        lines.append('load ' + wline(pkey(session['id']), [] if budget is UNDEF else V(budget)))
    calls = []
    now = case['world'].get('clockStart', 1700000000000)
    for e in case['script']:
        if e['ev'] == 'advance':
            now += e['ms']
            continue
        op = e['op']
        a = [jv_decode(x) for x in e['args']]
        ref = (a[1] if len(a) > 1 else UNDEF) if op == 'stopAutoContinue' else a[0]
        if op in AUTO:
            caller = pkey(a[0]) if op == 'stopAutoContinue' else []
            packet = to_b(a[1]) if op == 'recordAutoContinue' and len(a) > 1 and a[1] is not UNDEF else []
            line = 'auto ' + wline([AUTO[op]], V(ref), sid_of(ref), caller, packet)
        elif op == 'setCard':
            packets = [to_b(a[i]) if i < len(a) else [] for i in (1, 2, 3)]
            line = 'set ' + wline([now], V(ref), sid_of(ref), *packets)
        else:
            line = 'q ' + wline([QUERY[op]], V(ref), sid_of(ref))
        lines.append(line)
        if op == 'setCard':
            lines.append('dump')
        calls.append((e['req'], op, ref))
    lines.append('dump')
    return lines, calls


def judge(exp, raw, calls, loads, prefix):
    problems = []
    if raw[0] != 'ok' or any(line != 'ok 1' for line in raw[1:1 + loads]):
        problems.append('world setup failed')
    reqs = {req['id']: req for req in exp['requests']}
    events = defaultdict(list)
    for event in exp['trace']:
        if event['c'] in COUNTED:
            events[event.get('r')].append(event)
    final = [jv_decode(x['v']) for x in exp['finalSessions']]
    final_brefs = {s['id']: s['budget']['bref'] for s in final if isinstance(s.get('budget'), dict) and 'bref' in s['budget']}
    it = iter(raw[1 + loads:-1])
    for req, op, ref in calls:
        line = next(it)
        after = next(it) if op == 'setCard' else None
        try:
            w = parse_ok(line)
            if op in AUTO:
                finds, saved, changed = w[:3]
                now, at = 0, 3
            elif op == 'setCard':
                now, saved, changed, finds = w[:4]
                at = 4
            else:
                finds, now, saved, changed, at = w[0], 0, 0, 0, 1
            got, end = from_v(w, at)
            expected = reqs[req]
            brefs = []
            want = strip_brefs(jv_decode(expected['value']), brefs)
            if expected['terminal'] != 'ok' or canon(got) != canon(want):
                problems.append(f'{req} {op} value got {canon(got)[:180]} want {canon(want)[:180]}')
            sid = ref.get('sessionId') if isinstance(ref, dict) else None
            if any(final_brefs.get(sid) != b for b in brefs):
                problems.append(f'{req} {op} budget reference mismatch')
            actual = dict(zip(COUNTED, (finds, now, saved, changed)))
            hist = Counter(t['c'] for t in events[req])
            wanted = {k: hist[k] for k in COUNTED}
            if actual != wanted:
                problems.append(f'{req} {op} effects got {actual}, want {wanted}')
            if op == 'stopAutoContinue' and saved:
                saves = [t for t in events[req] if t['c'] == 'save']
                owner = None
                if saves and saves[0].get('a'):
                    session = jv_decode(saves[0]['a'][0])
                    if isinstance(session, dict) and 'sref' in session:
                        session = session.get('v')
                    if isinstance(session, dict):
                        owner = session.get('id')
                if isinstance(owner, str) and w[end:] != pkey(owner):
                    problems.append(f'{req} {op} saved owner words {w[end:]}, want {owner!r}')
                if saves and [t['c'] for t in events[req]][-2:] != ['save', 'changed']:
                    problems.append(f'{req} {op} oracle effect order changed')
            elif end != len(w):
                problems.append(f'{req} {op} trailing result words')
            if after is not None:
                blobs = dict(parse_dump(parse_ok(after)))
                if isinstance(got, dict) and got.get('ok') is True and blobs.get(sid) != w[17:end]:
                    problems.append(f'{req} setCard returned budget differs from the resident words')
        except (ValueError, KeyError, IndexError, TypeError) as ex:
            problems.append(f'{req} {op} malformed response or expectation: {str(ex)[:200]}')
    try:
        actual = []
        for sid, blob in parse_dump(parse_ok(raw[-1])):
            session = {'id': sid}
            if blob:
                session['budget'], end = from_v(blob)
                if end != len(blob):
                    raise ValueError(f'trailing budget words for {sid!r}')
            actual.append(session)
        wanted = [strip_brefs(s, []) for s in final]
        if canon(actual) != canon(wanted):
            problems.append(f'finalSessions got {canon(actual)[:250]}, want {canon(wanted)[:250]}')
    except (ValueError, IndexError) as ex:
        problems.append(f'finalSessions malformed: {str(ex)[:200]}')
    return ('prefix-fail' if problems else 'prefix-ok') if prefix else ('fail' if problems else 'exact'), problems


def observations(cases):
    full = oracle.run_oracle(cases)
    replay, flags = zip(*(cut_case(c) for c in cases))
    prefixes = iter(oracle.run_oracle([c for c, cut in zip(replay, flags) if cut]) if any(flags) else [])
    return [(c, next(prefixes) if cut else obs, cut) for c, obs, cut in zip(replay, full, flags)]


def replay(harness, rows):
    prepared = [case_lines(c) for c, _, _ in rows]
    lines = [line for ls, _ in prepared for line in ls]
    raw = run([harness], data='\n'.join(lines) + '\n').splitlines()
    if len(raw) != len(lines):
        raise RuntimeError(f'expected {len(lines)} harness lines, received {len(raw)}')
    pos = 0
    counts = Counter()
    for (case, exp, prefix), (ls, calls) in zip(rows, prepared):
        cls, messages = judge(exp, raw[pos:pos + len(ls)], calls, len(case['world'].get('sessions', [])), prefix)
        pos += len(ls)
        counts[cls] += 1
        if messages and counts[cls] <= 5:
            print(f'  {case["id"]} {cls}: {"; ".join(messages[:3])}', flush=True)
    return counts


def hdiff(harness, cases):
    lines = [line for case in cases for line in case_lines(case)[0]]
    data = '\n'.join(lines) + '\n'
    p = run([harness], data=data).splitlines()
    h = run([harness, '--hand'], data=data).splitlines()
    if len(p) == len(h) == len(lines) and p == h and all(s.startswith('ok') for s in p):
        print(f'hdiff identical ({len(cases)} cases, {len(lines)} lines)')
        return True
    for i, (a, b) in enumerate(zip(p, h)):
        if a != b:
            print(f'hdiff divergence line {i + 1}: {lines[i][:100]}\n  {a[:160]}\n  {b[:160]}')
            break
    return False


MUTANTS = {
    'caller-save': ('result_words(inout out, finds, 1, owner)', 'result_words(inout out, finds, 1, sid)', 2),
    'non-open-fallback': ('if pointed >= 0 { return stop_owner',
                          'if pointed >= 0 && string_is(st, crate::bud_card::obj_get(st, crate::bud_map::blob_off(st, pointed), "state"), "open") { return stop_owner', 1),
    'count-limit': ('  let written: i64 = put_jv(inout words, prefix, packet, 0, 3);',
                    '  let written: i64 = put_jv(inout words, prefix, packet, 0, 3);\n'
                    '  let count: i64 = crate::bud_card::obj_get(words, prefix, "count");\n'
                    '  if count >= 0 && count < 1099511627776 && rd(words, count) == 5 && rd(words, count + 1) > 2 { return result_words(inout out, 1, 3, empty); }', 1),
    'other-record': ('if string_matches(st, record_sid, sid) {',
                     'if record_sid >= 0 {', 1),
}


def perf(harness, rounds):
    node = [common.NODE, '--experimental-strip-types', '--no-warnings', '--import',
            (BUDGET / 'reference-hooks.mjs').as_uri(), HERE / 'perf_ts.mjs']
    print('item scale size  Pruv ns/op  hand ns/op  Pruv alloc/op  hand alloc/op  TS ns/op')
    for item in ('R', 'C', 'T', 'S'):
        for scale, size in zip(('S', 'M', 'L'), (100, 1000, 10000)):
            reps = 2048 if item == 'R' else 8192
            results = defaultdict(list)
            for round_no in range(rounds):
                order = ('perf', 'hand') if round_no % 2 == 0 else ('hand', 'perf')
                data = ''.join(f'{lane}{item} {size} {reps}\n' for lane in order)
                lines = run([harness], data=data).splitlines()
                if len(lines) != len(order):
                    raise RuntimeError(f"bad timing output: {lines}")
                for lane, line in zip(order, lines):
                    metric, ns, _, count, allocs = line.split()
                    assert (metric, count) == ('ns', 'alloc')
                    results[lane].append((float(ns), int(allocs) / reps))
                if item in ('R', 'T', 'S'):
                    line = run([*node, item, size, reps], env=common.TS_ENV())
                    results['TS'].append(float(line.split()[1]))
            p, h = (statistics.median(x[0] for x in results[lane]) for lane in ('perf', 'hand'))
            pa, ha = (statistics.median(x[1] for x in results[lane]) for lane in ('perf', 'hand'))
            ts = f'{statistics.median(results["TS"]):.1f}' if results['TS'] else '—'
            print(f'{item} {scale:>5} {size:5} {p:11.1f} {h:11.1f} {pa:14.3f} {ha:14.3f} {ts:>10}', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pairs', default='py,rs')
    parser.add_argument('--count', type=int, default=300)
    parser.add_argument('--seed', type=int, default=20261117)
    parser.add_argument('--no-build', action='store_true')
    action = parser.add_mutually_exclusive_group()
    action.add_argument('--mutants', action='store_true')
    action.add_argument('--hdiff', action='store_true')
    action.add_argument('--perf', type=int, metavar='ROUNDS')
    args = parser.parse_args()
    pairs = args.pairs.split(',')
    if any(p not in PAIRS for p in pairs) or args.count <= 0 or (args.perf is not None and args.perf <= 0):
        parser.error('use py,rs pairs and positive counts')
    (WORK / 'tmp').mkdir(parents=True, exist_ok=True)
    fixtures = [common.strip_case(x) for x in common.load_group('L')]
    generated = q7p(args.seed, args.count)
    if args.mutants:
        expected = observations(generated + fixtures)
        source = (BUDGET / 'bud_auto.pruv').read_text()
        good = True
        for mutant, (old, new, occurrences) in MUTANTS.items():
            assert source.count(old) == occurrences, mutant
            h = build(pairs[-1], mutant, source.replace(old, new))
            kills = replay(h, expected)['fail']
            print(f'{mutant}: {kills} fail', flush=True)
            good &= kills > 0
        return 0 if good else 1
    harnesses = {}
    for pair in pairs:
        h = WORK / 'base' / pair / 'harness'
        harnesses[pair] = h if args.no_build and h.exists() else build(pair)
        print(f'[{PAIRS[pair]}] built', flush=True)
    if args.hdiff:
        corpus = generated + fixtures + crafted() + crafted(big=True)
        checks = []
        for pair, h in harnesses.items():
            print(f'[{PAIRS[pair]}] ', end='')
            checks.append(hdiff(h, corpus))
        return 0 if all(checks) else 1
    if args.perf:
        perf(harnesses[pairs[-1]], args.perf)
        return 0
    good = True
    for pair, h in harnesses.items():
        problems = fixed_checks(h)
        for msg in problems[:5]:
            print(f'  [{PAIRS[pair]}] {msg}', flush=True)
        good &= not problems
    suites = {'Q7': q_gen.gen(args.seed, ['Q7'], count=args.count), 'Q7P': generated, 'L': fixtures, 'crafted': crafted()}
    expected = {name: observations(cases) for name, cases in suites.items()}
    for pair, h in harnesses.items():
        for name, rows in expected.items():
            counts = replay(h, rows)
            print(f'[{PAIRS[pair]}] {name}: ' + ' '.join(f'{k}={counts[k]}' for k in ('exact', 'prefix-ok', 'prefix-fail', 'fail')), flush=True)
            good &= not (counts['fail'] or counts['prefix-fail'])
            if name != 'Q7':
                good &= counts['exact'] == len(rows)
    return 0 if good else 1


if __name__ == '__main__':
    sys.exit(main())
