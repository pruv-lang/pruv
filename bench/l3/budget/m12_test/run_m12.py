#!/usr/bin/env python3
"""Replay M12 against the pinned oracle, check mutants and hand parity, or time both lanes."""
import sys
sys.dont_write_bytecode = True

import argparse
from collections import Counter, defaultdict
from copy import deepcopy
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess

HERE = Path(__file__).resolve().parent
BUDGET = HERE.parent
REPO = HERE.parents[3]
CORPUS = REPO / 'work' / 'p29'
WORK = HERE / 'target' / 'p31t9'
sys.path[:0] = [str(BUDGET), str(BUDGET / 'm11_test')]

import common
import oracle
import q_gen
from compare import UNDEF, jv_decode, jv_encode
from run_m11 import (MODULES, PAIRS, PY_DEFAULT, RS_DEFAULT, QUERY, COUNTED, V, canon,
                     from_units, from_v, parse_dump, parse_ok, pkey, sid_of, strip_brefs, to_b,
                     tool_env, wline)

AUTO = {'clearAutoContinue': 1, 'recordAutoContinue': 2, 'stopAutoContinue': 3}
LIFE = {'bootRestore': 1, 'forgetSession': 2, 'dispose': 3, 'runStarted': 4,
        'runWaiting': 5, 'runEnded': 6, 'attach': 8, 'charge': 9, 'halt': 10}
LIFE_PERF = {item: sizes for items, sizes in (
    ('FD', (10, 50, 100)), ('O', (100, 1000, 5000)), ('BAHU', (100, 1000, 10000)))
    for item in items}
SUPPORTED = {*AUTO, *QUERY, *LIFE, 'setCard'}
EFFECTS = {
    1: 'findSession', 2: 'now', 3: 'save', 4: 'changed', 5: 'unit', 6: 'uuid',
    7: 'runningDelegationIds', 8: 'cancelRecovery', 9: 'stopMain',
    10: 'stopDelegation', 11: 'stopAttachedTurns', 12: 'timer.set', 13: 'timer.clear',
    15: 'holdQueued', 16: 'announce', 17: 'timer.unref', 18: 'advance', 19: 'timer.fire',
}


def run(cmd, *, data=None, env=None, cwd=BUDGET):
    env = dict(tool_env() if env is None else env)
    env.update(TMPDIR=str(WORK / 'tmp'), PYTHONDONTWRITEBYTECODE='1')
    p = subprocess.run([str(x) for x in cmd], cwd=cwd, env=env, input=data,
                       capture_output=True, text=True)
    if p.returncode:
        raise RuntimeError(f'{cmd[0]} exited {p.returncode}\n{p.stdout[-4000:]}\n{p.stderr[-4000:]}')
    return p.stdout


def typed_harness(main):
    # Reuse the flat protocol, scheduler and hand lane in the generated harness.
    adapter = HERE / 'harness_typed.rs'
    main = main.replace(
        '#[path = "@SOL@"] mod sol;',
        f'#[path = "@SOL@"] mod generated;\n#[path = "{adapter}"] mod sol;')
    main = main.replace('    Pruv(World),', '    Pruv(sol::World),', 1)
    main = main.replace('Self::Pruv(World::new())', 'Self::Pruv(sol::World::new())', 1)
    main = main.replace(
        'Lane::Pruv(w) => format!("ok {} {}", w.grows, (w.st.len() as i64).min(w.st[11]) - w.st[10]),',
        'Lane::Pruv(_) => String::from("err 2"),')
    start = main.index('// Pruv reserves space')
    end = main.index('#[derive(Clone)]', start)
    main = main[:start] + main[end:]
    start = main.index('#[derive(Clone)]\nstruct World')
    end = main.index('fn words(', start)
    main = main[:start] + main[end:]
    main = main.replace('    Pruv(World),', '    Pruv(sol::St),', 1)
    main = main.replace('Clocks::Pruv(World::new())', 'Clocks::Pruv(sol::new_state())')
    main = main.replace(
        'Self::Pruv(w) => w.call(&|st, out| sol::m12_observe(st, out)).map(<[_]>::to_vec),',
        'Self::Pruv(w) => Ok(sol::observe_clocks(w)),')
    main = main.replace(
        'Self::Pruv(w) => w.call(&|st, out| sol::m12_seed_tool(st, turn, activity, out)).map(|v| v[0]),',
        'Self::Pruv(w) => sol::seed_tool(w, turn, activity),')
    for operation in ('life', 'fire'):
        start = main.index('(Self::Pruv(clocks), lane) => lifecycle(')
        end = main.index('(Self::Hand(clocks)', start)
        invoke = ('sol::m12_life(&mut w.st, clocks, op, rf, sid, arg, now)' if operation == 'life'
                  else 'sol::m12_fire(&mut w.st, clocks, sid, bid, budget_first, now)')
        main = main[:start] + f'(Self::Pruv(clocks), Lane::Pruv(w)) => {invoke},\n            ' + main[end:]
    return main


def build(pair, variant='base', source=None, *, typed=False):
    out = WORK / variant / pair
    out.mkdir(parents=True, exist_ok=True)
    root = out / 'root.pruv'
    shutil.copy(HERE / ('root_m12_typed.pruv' if typed else 'root.pruv'), root)
    if typed:
        for name in ('bud_tstate.pruv', 'bud_tlife.pruv'):
            shutil.copy(BUDGET / name, out)
    for name in (*MODULES, 'bud_clock.pruv', 'bud_charge.pruv', 'bud_vwords.pruv',
                 'bud_effect.pruv', 'bud_cleanup.pruv', 'bud_vlife.pruv'):
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
    rlib = out / 'libm12_facts.rlib'
    run(['rustc', '--crate-type', 'rlib', '--edition', '2021',
         HERE / 'm12_facts.rs', '-o', rlib])
    externs = ['--rust-extern', f'm12_facts={rlib}']
    if typed:
        externs += ['--rust-extern', f'bud_facts={rlib}']
    run([*cmd, *externs], cwd=REPO)
    main = out / 'main.rs'
    harness = (HERE / 'harness.rs').read_text()
    if typed:
        harness = typed_harness(harness)
    main.write_text(harness
                    .replace('@SOL@', str(sol)).replace('@HAND@', str(HERE / 'hand.rs'))
                    .replace('@PERF_LIFE@', str(HERE / 'perf_life.rs')))
    typed_flags = ['--cfg', 'm12_typed', '--extern', f'bud_facts={rlib}'] if typed else []
    run(['rustc', '-O', '--edition', '2021', '-A', 'warnings', '--extern',
         f'm12_facts={rlib}', *typed_flags, main, '-o', out / 'harness'])
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


def fixed_checks(harness, *, typed=False):
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
    if typed:
        # The first budgetId controls the timer's reference order, but the
        # last duplicate is still the value resolved by runStarted.
        base = budget_fixture('b1')
        duplicate_ref = V({'budgetId': 'stale', 'sessionId': sid})
        duplicate_ref[1] += 1
        duplicate_ref += V({'budgetId': 'b1'})[2:]
        data = '\n'.join([
            'init', 'world ' + wline([1700000000000, 1, 1, 0], V(base['limits']), [0]),
            'load ' + wline(pkey(sid), V(base)),
            'life ' + wline([0, 4], duplicate_ref, pkey(sid), V({'run': 'run'})),
            'advance 60000', 'observe 0', 'timers', 'dump',
        ]) + '\n'
        pruv, hand = (run([harness, *lane], data=data).splitlines() for lane in ([], ['--hand']))
        if pruv != hand or len(pruv) != 8 or any(not row.startswith('ok') for row in hand):
            problems.append('duplicate budgetId: timer fire differs from hand')
        print(f'fixed checks: dup auto 4 runs, timer 1, {len(problems)} problems', flush=True)
        return problems
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
        if event['ev'] == 'call' and event['op'] not in SUPPORTED:
            return {**case, 'script': case['script'][:i]}, True
    return case, False


def case_lines(case, legacy=False):
    world = case['world']
    instances = case.get('instances', ['A'])
    lines = ['init']
    now = world.get('clockStart', 1700000000000)
    if not legacy:
        uuids = [pkey(jv_decode(x)) for x in world.get('uuids', [])]
        running = [[len(ids), *(word for sid in ids for word in pkey(sid))]
                   for ids in map(jv_decode, world.get('deps', {}).get('runningDelegationIds', []))]
        lines.append('world ' + wline(
            [now, len(instances), int(world.get('timerHasUnref', True)), int('uuids' in world)],
            V(jv_decode(world.get('unit', {'undef': 1}))), [len(uuids)], *uuids, *running))
    for raw in world.get('sessions', []):
        session = jv_decode(raw)
        budget = session.get('budget', UNDEF)
        lines.append('load ' + wline(pkey(session['id']), [] if budget is UNDEF else V(budget)))
    calls = []
    advance = 0
    for e in case['script']:
        if e['ev'] == 'putSession':
            session = jv_decode(e['session'])
            lines.append('load ' + wline(pkey(session['id']), V(session['budget'])))
            calls.append((None, 'putSession', None))
            continue
        if e['ev'] == 'advance':
            now += e['ms']
            if not legacy:
                lines.append('advance ' + wline([e['ms']]))
                calls.append((advance, 'advance', None))
                advance += 1
            continue
        op = e['op']
        i = instances.index(e.get('i', 'A'))
        a = [jv_decode(x) for x in e['args']]
        ref = a[0] if a else UNDEF
        if op == 'noteTool':
            assert ref['sessionId'] == 'missing-fixture-session'
            lines.append('seed-tool ' + wline([i], pkey(a[1]), pkey(a[2])))
            calls.append((e['req'], op, ref))
            continue
        if op in AUTO:
            if op == 'stopAutoContinue':
                ref = a[1] if len(a) > 1 else UNDEF
            caller = pkey(a[0]) if op == 'stopAutoContinue' else []
            packet = to_b(a[1]) if op == 'recordAutoContinue' and len(a) > 1 and a[1] is not UNDEF else []
            line = 'auto ' + wline([AUTO[op]], V(ref), sid_of(ref), caller, packet)
        elif op == 'setCard':
            packets = [to_b(a[j]) if j < len(a) else [] for j in (1, 2, 3)]
            line = 'set ' + wline([now], V(ref), sid_of(ref), *packets)
        elif op in QUERY:
            line = 'q ' + wline([QUERY[op]], V(ref), sid_of(ref))
        else:
            sid, arg = sid_of(ref), UNDEF
            if op in ('bootRestore', 'attach'):
                sid, ref = pkey(a[0]['sess']), UNDEF
                if op == 'attach':
                    arg = {'origin': a[1], 'size': a[2] if len(a) > 2 else UNDEF}
            elif op == 'forgetSession':
                sid = pkey(a[0])
            elif op in ('runStarted', 'runWaiting', 'runEnded'):
                arg = {'run': a[1], 'waiting': a[2] if len(a) > 2 else UNDEF}
            elif op in ('charge', 'halt'):
                arg = a[1]
            line = 'life ' + wline([i, LIFE[op]], V(ref), sid, V(arg))
        if not legacy and op not in LIFE:
            line = 'trace-' + line + ' | ' + str(i)
        lines.append(line)
        if op == 'setCard':
            lines.append('dump')
        calls.append((e['req'], op, ref))
    if not legacy:
        lines.extend(f'observe {i}' for i in range(len(instances)))
        lines.append('timers')
    lines.append('dump')
    return lines, calls


def token_id(tokens, token):
    return tokens.setdefault(token, len(tokens) + 1)


def session_id(value):
    return value['v']['id'] if 'sref' in value else value['id']


def session_snapshot(value):
    if value is UNDEF:
        return UNDEF
    session = strip_brefs(value.get('v', value), [])
    return {key: session[key] for key in ('id', 'budget') if key in session}


def trace_groups(trace, tokens):
    request_brefs = defaultdict(dict)
    groups = defaultdict(list)
    advance = -1
    for t in trace:
        name, instance, req = t['c'], t['i'], t['r']
        if name in ('call.start', 'call.end', 'putSession'):
            continue
        if name == 'advance':
            advance += 1
        a = jv_decode(t.get('a', {'undef': 1}))
        o = jv_decode(t.get('o', {'undef': 1}))
        session = UNDEF
        if name == 'findSession':
            session = o
            data = [a[0], session_snapshot(session)]
        elif name in ('stopMain', 'stopAttachedTurns'):
            data = a[0]
        elif name in ('save', 'announce', 'cancelRecovery', 'holdQueued'):
            session = a[0]
            address = session_id(session)
            data = [[address, a[1]] if name == 'holdQueued' else address, session_snapshot(session)]
        elif name == 'runningDelegationIds':
            data = [a[0], o]
        elif name in ('now', 'unit', 'uuid'):
            data = o
        elif name == 'advance':
            data = a['ms']
        elif name == 'timer.set':
            data = [token_id(tokens, a['token']), a['delay'], a['due']]
        elif name.startswith('timer.') and 'token' in a:
            data = [token_id(tokens, a['token'])]
        elif name == 'changed':
            data = UNDEF
        else:
            data = a
        if req is not None and session is not UNDEF:
            snapshot = session.get('v', session)
            budget = snapshot.get('budget')
            request_brefs[req][snapshot['id']] = budget.get('bref') if isinstance(budget, dict) else None
        groups[advance if req is None else req].append([name, instance, data])
    return groups, request_brefs


def trace_response(line, instances, tokens):
    result, log = line.split('|', 1)
    w = parse_ok(result)
    log = [int(x) for x in log.split()]
    events = []
    pos = 0
    while pos < len(log):
        code, i, n = log[pos:pos + 3]
        pos += 3
        if n < 0 or pos + n > len(log):
            raise ValueError('invalid effect payload length')
        payload = log[pos:pos + n]
        pos += n
        name = EFFECTS[code]
        if name.startswith('timer.'):
            data = [token_id(tokens, payload[0]), *payload[1:]]
        elif name == 'advance':
            data, = payload
        elif not payload:
            data = UNDEF
        else:
            data, end = from_v(payload)
            if end != n:
                raise ValueError('trailing effect payload words')
        events.append([name, instances[i] if i >= 0 else None, data])
    return w, events


def unpack_key(words):
    n, *packed = words
    if n < 0 or len(packed) != (n + 3) // 4:
        raise ValueError('invalid packed key')
    units = [(word >> shift) & 65535 for word in packed for shift in (48, 32, 16, 0)]
    return from_units(units[:n])


class Frame:
    def __init__(self, words):
        self.words = words
        self.pos = 0

    def take(self, n):
        end = self.pos + n
        if n < 0 or end > len(self.words):
            raise ValueError('invalid state frame length')
        result = self.words[self.pos:end]
        self.pos = end
        return result

    def word(self):
        return self.take(1)[0]

    def count(self):
        n = self.word()
        if not 0 <= n <= len(self.words) - self.pos:
            raise ValueError('invalid state frame count')
        return n

    def key(self):
        return from_units(self.take(self.count()))

    def done(self):
        if self.pos != len(self.words):
            raise ValueError('trailing state words')


def final_instance_state(raw, instances, tokens):
    f = Frame(parse_ok(raw[-2].split('|', 1)[0]))
    timers = []
    for _ in range(f.count()):
        token, delay, due, i = f.take(4)
        timers.append({'token': tokens[token], 'delay': delay, 'due': due, 'i': instances[i]})
    named = {}
    for _ in range(f.count()):
        i, n = f.take(2)
        bid = unpack_key(f.take(n))
        named[(instances[i], bid)] = tokens[f.word()]
    f.done()
    clocks, seen = {}, {}
    for instance, line in zip(instances, raw[-2 - len(instances):-2]):
        f = Frame(parse_ok(line))
        if f.word():
            raise ValueError('session entry in volatile store')
        clocks[instance] = []
        for _ in range(f.count()):
            bid, sid = f.key(), f.key()
            present, since, armed, active_count = f.take(4)
            runs = [[f.key(), {'waiting': bool(f.word())}] for _ in range(f.count())]
            if active_count != sum(not fields['waiting'] for _, fields in runs):
                raise ValueError('clock active count differs from its runs')
            clock = {'sessionId': sid, 'runs': runs}
            if present:
                clock['since'] = since
            if armed:
                clock['timer'] = named.pop((instance, bid))
            clocks[instance].append([bid, clock])
        seen[instance] = [[f.key(), [f.key() for _ in range(f.count())]] for _ in range(f.count())]
        f.done()
    if named:
        raise ValueError('scheduler holds unowned named timers')
    return clocks, seen, timers


def expected_instance_state(exp, tokens):
    clocks = {i: jv_decode(w) for i, w in exp['finalClocks'].items()}
    for entries in clocks.values():
        for _, clock in entries:
            if 'timer' in clock:
                clock['timer'] = tokens[clock['timer']]
    seen = {i: jv_decode(w) for i, w in exp['finalToolSeen'].items()}
    timers = [{**t, 'token': tokens[t['token']], 'delay': jv_decode(t['delay']), 'due': jv_decode(t['due'])}
              for t in exp['activeTimers']]
    return clocks, seen, timers


def judge(exp, raw, calls, loads, instances, prefix):
    problems = []
    if exp['driver'] or exp['excluded']:
        problems.append(f"oracle driver/domain issue: {exp['driver']} {exp['excluded']}")
    if raw[:2] != ['ok', 'ok'] or any(line != 'ok 1' for line in raw[2:2 + loads]):
        problems.append('world setup failed')
    reqs = {req['id']: req for req in exp['requests']}
    wanted_tokens, actual_tokens = {}, {}
    events, request_brefs = trace_groups(exp['trace'], wanted_tokens)
    final = [jv_decode(x['v']) for x in exp['finalSessions']]
    it = iter(raw[2 + loads:-2 - len(instances)])
    for req, op, ref in calls:
        line = next(it)
        after = next(it) if op == 'setCard' else None
        try:
            if op == 'putSession':
                if line != 'ok 0':
                    problems.append('fixture did not replace the resident session')
                continue
            if op == 'noteTool':
                expected = events[req]
                valid = not expected or expected == [
                    ['findSession', reqs[req]['instance'], [ref['sessionId'], UNDEF]]
                ]
                if not valid or line != f'ok {len(expected)}' or reqs[req]['terminal'] != 'ok':
                    problems.append(f'{req} fixture toolSeen insert got {line}, want {expected}')
                continue
            w, log = trace_response(line, instances, actual_tokens)
            wanted_log = events[req]
            if canon(log) != canon(wanted_log):
                problems.append(f'{req} {op} ordered effects got {canon(log)[:300]}, want {canon(wanted_log)[:300]}')
            if op == 'advance':
                if w:
                    problems.append(f'advance {req} trailing result words')
                continue
            if op in LIFE:
                at = 0
                counted = Counter(t[0] for t in log)
                finds, now, saved, changed = (counted[k] for k in COUNTED)
            elif op in AUTO:
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
            if expected['op'] != op or expected['terminal'] != 'ok' or canon(got) != canon(want):
                problems.append(f'{req} {op} value got {canon(got)[:180]} want {canon(want)[:180]}')
            sid = ref.get('sessionId') if isinstance(ref, dict) else None
            if any(request_brefs[req].get(sid) != b for b in brefs):
                problems.append(f'{req} {op} budget reference mismatch')
            actual = dict(zip(COUNTED, (finds, now, saved, changed)))
            hist = Counter(t[0] for t in wanted_log)
            wanted = {k: hist[k] for k in COUNTED}
            if actual != wanted:
                problems.append(f'{req} {op} effects got {actual}, want {wanted}')
            if op == 'stopAutoContinue' and saved:
                saves = [t for t in wanted_log if t[0] == 'save']
                owner = saves[0][2][0] if saves else None
                if isinstance(owner, str) and w[end:] != pkey(owner):
                    problems.append(f'{req} {op} saved owner words {w[end:]}, want {owner!r}')
                if saves and [t[0] for t in wanted_log if t[0] in COUNTED][-2:] != ['save', 'changed']:
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
    try:
        actual = final_instance_state(raw, instances, actual_tokens)
        wanted = expected_instance_state(exp, wanted_tokens)
        for label, got, want in zip(('finalClocks', 'finalToolSeen', 'activeTimers'), actual, wanted):
            if canon(got) != canon(want):
                problems.append(f'{label} got {canon(got)[:250]}, want {canon(want)[:250]}')
    except (ValueError, IndexError, KeyError) as ex:
        problems.append(f'instance state malformed: {str(ex)[:200]}')
    return ('prefix-fail' if problems else 'prefix-ok') if prefix else ('fail' if problems else 'exact'), problems


def observations(cases, *, allow_prefix=True):
    full = oracle.run_oracle(cases)
    if not allow_prefix:
        assert len(full) == len(cases)
        return [(case, obs, False) for case, obs in zip(cases, full)]
    replay, flags = zip(*(cut_case(c) for c in cases))
    prefixes = iter(oracle.run_oracle([c for c, cut in zip(replay, flags) if cut]) if any(flags) else [])
    return [(c, next(prefixes) if cut else obs, cut) for c, obs, cut in zip(replay, full, flags)]


def budget_fixture(bid, minutes=1, *, unlimited=False):
    return {
        'id': bid, 'openedAt': '2023-11-14T22:13:20.000Z', 'origin': 'user', 'state': 'open',
        'limits': {'timeMinutes': minutes, 'delegations': 2, 'commands': 3, 'reviews': 4, 'checks': 5},
        'used': dict.fromkeys(('activeMs', 'delegations', 'commands', 'reviews', 'checks',
                               'tools', 'failures', 'retries'), 0),
        **({'unlimited': True} if unlimited else {}),
    }


def cleanup_case(item, size):
    sessions, script, clock_ids, armed_ids = {}, [], [], []
    matching = size // 2

    def call(op, *args):
        script.append({'ev': 'call', 'req': f'r{len(script)}', 'op': op, 'args': args})

    for i in range(size):
        sid = f's{i}'
        if (item == 'O' and i == matching) or (item != 'O' and i % 2 == 0):
            sid = 'owner'
        armed = i % 3 != 0 or (item == 'O' and sid == 'owner')
        bid = f'b{i}'
        # Vary deadlines independently of insertion order and keep some clocks unarmed.
        session = {'id': sid, 'budget': budget_fixture(bid, size - i, unlimited=not armed)}
        if sid in sessions:
            script.append({'ev': 'putSession', 'session': session})
        else:
            sessions[sid] = session
        call('runStarted', {'sessionId': sid, 'budgetId': bid}, 'run')
        clock_ids.append((bid, sid))
        if armed:
            armed_ids.append((bid, sid))
    if item != 'O':
        stale = {'sessionId': 'missing-fixture-session', 'budgetId': 'stale'}
        for turn, activity in [('z-turn', 'a'), ('b-turn', 'c'), ('z-turn', 'b'), ('z-turn', 'a')]:
            call('noteTool', stale, turn, activity)
    if item == 'D':
        call('dispose')
    else:
        call('forgetSession', 'owner')
    return {'id': f'{item}={size}', 'world': {'sessions': list(sessions.values())}, 'script': script}, clock_ids, armed_ids


def cleanup_checks(harnesses):
    fixtures = [cleanup_case(item, size) for item, sizes in (
        ('F', (10, 50, 100)), ('D', (10, 50, 100)), ('O', (100, 1000, 5000)))
        for size in sizes]
    rows = observations([case for case, _, _ in fixtures], allow_prefix=False)
    good = True
    for (case, clocks, armed), (_, obs, _) in zip(fixtures, rows):
        dispose = case['id'].startswith('D')
        removed = lambda sid: dispose or sid == 'owner'
        expected_survivors = [bid for bid, sid in clocks if not removed(sid)]
        actual_survivors = [bid for bid, _ in jv_decode(obs['finalClocks']['A'])]
        registered = [jv_decode(t['a'])['token'] for t in obs['trace'] if t['c'] == 'timer.set']
        cleared = [jv_decode(t['a'])['token'] for t in obs['trace'] if t['c'] == 'timer.clear']
        assert len(registered) == len(armed)
        assert cleared == [token for token, (_, sid) in zip(registered, armed) if removed(sid)], case['id']
        assert actual_survivors == expected_survivors, case['id']
        if case['id'].startswith('O'):
            assert len(actual_survivors) == len(clocks) - 1
        for pair, harness in harnesses.items():
            one = [(case, obs, False)]
            p = replay(harness, one)['exact'] == 1
            h = replay(harness, one, hand=True)['exact'] == 1
            print(f'[{PAIRS[pair]}] {case["id"]}: Pruv={"exact" if p else "fail"} '
                  f'hand={"exact" if h else "fail"} removed={len(clocks) - len(actual_survivors)} '
                  f'cleared={len(cleared)}', flush=True)
            good &= p and h
    return good


def lifecycle_cases():
    cases = []
    for minutes, action in [(1, 'halt'), (1, 'charge'), (1, 'advance'), (0, 'start'),
                            (36000, 'advance')]:
        s = {'id': 'life', 'budget': budget_fixture('clock', minutes)}
        ref = {'budgetId': 'clock', 'sessionId': 'life'}
        script = [{'ev': 'call', 'req': 'start', 'op': 'runStarted', 'args': [ref, 'run']}]
        if action == 'advance':
            script.append({'ev': 'advance', 'ms': 60000 if minutes == 1 else 2})
        elif action != 'start':
            arg = 'reviews' if action == 'halt' else {'commands': 4}
            script.append({'ev': 'call', 'req': action, 'op': action, 'args': [ref, arg]})
        cases.append({'id': f'lifecycle-{action}-{minutes}', 'script': script, 'world': {
            'sessions': [s], 'deps': {'runningDelegationIds': [['dup', 'other', 'dup']]},
        }})
    budget = budget_fixture('b1')
    ref = {'sessionId': 'life', 'budgetId': 'b1'}
    budget['used']['delegations'] = -(2 ** 53 - 1)
    budget['limits']['commands'] = 0
    cases.append({'id': 'lifecycle-charge-limit-before-overflow', 'world': {
        'sessions': [{'id': 'life', 'budget': budget}],
    }, 'script': [{'ev': 'call', 'req': 'charge', 'op': 'charge',
                   'args': [ref, {'delegations': -1, 'commands': 1}]}]})
    cases.append({'id': 'lifecycle-current-before-reattach', 'world': {
        'sessions': [{'id': 'life', 'budget': {**budget_fixture('b1'), 'state': 'closed'}}],
        'unit': budget_fixture('b1')['limits'], 'uuids': ['b2'],
    }, 'script': [
        {'ev': 'call', 'req': 'current', 'op': 'current', 'args': [ref]},
        {'ev': 'call', 'req': 'attach', 'op': 'attach', 'args': [{'sess': 'life'}, 'user', 1]},
    ]})
    return cases


def audit_crafted(harness):
    """Use the V ABI where the pinned oracle collapses duplicate keys or admits errors."""
    sid = 's'
    bid = 'b'
    ref = {'sessionId': sid, 'budgetId': bid}
    run_arg = {'run': 'run'}
    unit = budget_fixture(bid)['limits']

    def world(limits=unit, uuids=()):
        return ['init', 'world ' + wline([1700000000000, 1, 1, 1], V(limits),
                                          [len(uuids)], *map(pkey, uuids))]

    def load(s, budget):
        return 'load ' + wline(pkey(s), budget)

    def call(op, s=sid, arg=UNDEF, reference=ref):
        return 'life ' + wline([0, op], V(reference), pkey(s), V(arg))

    def prepend(budget, fields):
        words, prefix = V(budget), V(fields)
        return [9, words[1] + prefix[1], *prefix[2:], *words[2:]]

    base = budget_fixture(bid)
    active = '2023-11-14T22:13:20.000Z'
    duplicate_active = prepend({**base, 'activeSince': active}, {'activeSince': 'stale'})
    closed = {**base, 'state': 'closed'}
    attach = {'origin': 'user', 'size': 2}
    def duplicate_charge_budget(commands):
        used = prepend({**base['used'], 'commands': commands}, {'commands': -8})
        budget = prepend({k: v for k, v in base.items() if k != 'used'},
                         {'id': 'stale', 'used': {'commands': -9}})
        budget[1] += 1
        return [*budget, *V({'used': UNDEF})[2:-1], *used]
    cases = [
        ('duplicate-activeSince-boot', world() + [load(sid, duplicate_active), call(1)],
         {}, {sid: V(base)}),
        ('duplicate-activeSince-settle', world() + [load(sid, V(base)), call(4, arg=run_arg),
            load(sid, duplicate_active), 'advance 5', call(6, arg=run_arg)],
         {}, {sid: V({**base, 'used': {**base['used'], 'activeMs': 5}})}),
        ('attach-overflow', world({**unit, 'commands': 2 ** 52}, [bid]) +
         [load(sid, []), call(8, arg=attach)], {3: 5}, {sid: []}),
        ('unknown-op', world() + [load(sid, V(base)), call(4, arg=run_arg),
                                  call(11), call(0), 'advance 60000'],
         {4: 2, 5: 2}, None),
        ('duplicate-uuid-clock', world(uuids=[bid, bid]) +
         [load(sid, []), load('other', []), call(8, arg=attach), call(4, arg=run_arg),
          call(8, 'other', attach), call(4, 'other', run_arg, {**ref, 'sessionId': 'other'})],
         {6: 2}, {sid: V({**base, 'limits': {k: 2 * v for k, v in unit.items()},
                       'activeSince': active}), 'other': []}),
        ('duplicate-uuid-resident', world(uuids=[bid]) + [load(sid, V(closed)), call(8, arg=attach)],
         {3: 2}, {sid: V(closed)}),
        ('duplicate-id-keys', world() + [load(sid, duplicate_charge_budget(0)),
            call(9, arg={'commands': 1})], {}, {sid: duplicate_charge_budget(1)}),
    ]
    passed = 0
    for name, lines, errors, expected in cases:
        lines = [*lines, 'observe 0', 'timers', 'dump']
        data = '\n'.join(lines) + '\n'
        p, h = [run([harness, *lane], data=data).splitlines() for lane in ([], ['--hand'])]
        ok = p == h and len(p) == len(lines) and all(
            result == f'err {errors[i]}' if i in errors else result.startswith('ok')
            for i, result in enumerate(p))
        if ok and expected is not None:
            ok = dict(parse_dump(parse_ok(p[-1]))) == expected
        if ok and name == 'unknown-op':
            # The timer still fires after both rejected calls.
            ok = from_v(dict(parse_dump(parse_ok(p[-1])))[sid])[0]['halt']['reason'] == 'time'
        passed += ok
        if not ok:
            print(f'  {name} fail: Pruv {str(p)[:300]} hand {str(h)[:300]}', flush=True)
    print(f'audit crafted / hdiff: {passed}/{len(cases)}', flush=True)
    return passed == len(cases)


def save_snapshot_repro(harness, row):
    case, exp, prefix = row
    lines, calls = case_lines(case)
    raw = run([harness], data='\n'.join(lines) + '\n').splitlines()
    args = (raw, calls, len(case['world'].get('sessions', [])), case.get('instances', ['A']), prefix)
    original, _ = judge(exp, *args)
    mutation = deepcopy(exp)
    save = next(t for t in mutation['trace'] if t['c'] == 'save' and t['r'] == 'r7')
    budget = dict(save['a'][0]['v']['obj'])['budget']['v']['obj']
    state = next(pair for pair in budget if pair[0] == 'state')
    assert state[1] == 'halted'
    state[1] = 'open'
    mutated, problems = judge(mutation, *args)
    killed = mutated == 'fail' and len(problems) == 1 and problems[0].startswith('r7 ')
    print(f'{case["id"]} r7 save snapshot: original={original}, mutated={mutated}', flush=True)
    return original == 'exact' and killed


def replay(harness, rows, *, hand=False):
    prepared = [case_lines(c) for c, _, _ in rows]
    lines = [line for ls, _ in prepared for line in ls]
    raw = run([harness, *(['--hand'] if hand else [])], data='\n'.join(lines) + '\n').splitlines()
    if len(raw) != len(lines):
        raise RuntimeError(f'expected {len(lines)} harness lines, received {len(raw)}')
    pos = 0
    counts = Counter()
    for (case, exp, prefix), (ls, calls) in zip(rows, prepared):
        cls, messages = judge(exp, raw[pos:pos + len(ls)], calls, len(case['world'].get('sessions', [])),
                              case.get('instances', ['A']), prefix)
        pos += len(ls)
        counts[cls] += 1
        if messages and counts[cls] <= 5:
            print(f'  {case["id"]} {cls}: {"; ".join(messages[:3])}', flush=True)
    return counts


def hdiff(harness, cases, *, legacy=False, label='Q7'):
    prepared = [case_lines(case, legacy=legacy)[0] for case in cases]
    lines = [line for batch in prepared for line in batch]
    data = '\n'.join(lines) + '\n'
    p = run([harness], data=data).splitlines()
    h = run([harness, '--hand'], data=data).splitlines()
    if len(p) != len(lines) or len(h) != len(lines):
        raise RuntimeError(f'hdiff {label}: expected {len(lines)} lines, got {len(p)} and {len(h)}')
    equal = 0
    reported = False
    offset = 0
    for case, batch in zip(cases, prepared):
        mismatch = next((i for i in range(offset, offset + len(batch))
                         if p[i] != h[i] or not p[i].startswith('ok')), None)
        if mismatch is None:
            equal += 1
        elif not reported:
            print(f'  {case["id"]}: {lines[mismatch][:100]}\n  Pruv {p[mismatch][:240]}\n  hand {h[mismatch][:240]}')
            reported = True
        offset += len(batch)
    print(f'hdiff {label} {equal}/{len(cases)} equal ({len(lines)} lines)', flush=True)
    return equal == len(cases)


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
            perf_samples(item, size, results)


def perf_samples(item, size, results):
    if os.environ.get('M12_PERF_RAW') == '1':
        print('raw ' + json.dumps({'item': item, 'size': size, **results}), flush=True)


def perf_result(line):
    metric, ns, checksum, count, allocs = line.split()
    if (metric, count) != ('ns', 'alloc'):
        raise ValueError(f'bad timing output: {line}')
    return float(ns), int(allocs), int(checksum)


def lifecycle_perf(harness, rounds, reps=64):
    for item, sizes in LIFE_PERF.items():
        for size in sizes:
            results = defaultdict(list)
            checksums = set()
            for round_no in range(rounds):
                order = ('perf', 'hand') if round_no % 2 == 0 else ('hand', 'perf')
                data = ''.join(f'{lane}{item} {size} {reps}\n' for lane in order)
                lines = run([harness], data=data).splitlines()
                if len(lines) != len(order):
                    raise ValueError(f'{item} {size}: expected {len(order)} result lines, got {len(lines)}')
                for lane, line in zip(order, lines):
                    ns, allocs, checksum = perf_result(line)
                    results[lane].append((ns, allocs / reps))
                    checksums.add(checksum)
            if len(checksums) != 1:
                raise ValueError(f'{item} {size}: inconsistent checksums {checksums}')
            p, h = (statistics.median(x[0] for x in results[lane]) for lane in ('perf', 'hand'))
            pa, ha = (statistics.median(x[1] for x in results[lane]) for lane in ('perf', 'hand'))
            perf_samples(item, size, results)
            yield item, size, p, h, pa, ha, checksums.pop()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pairs', default='py,rs')
    parser.add_argument('--count', type=int, default=300)
    parser.add_argument('--seed', type=int, default=20261117)
    parser.add_argument('--no-build', action='store_true')
    parser.add_argument('--typed', action='store_true')
    action = parser.add_mutually_exclusive_group()
    action.add_argument('--mutants', action='store_true')
    action.add_argument('--hdiff', action='store_true')
    action.add_argument('--cleanup-checks', action='store_true')
    action.add_argument('--perf', type=int, metavar='ROUNDS')
    args = parser.parse_args()
    pairs = args.pairs.split(',')
    if any(p not in PAIRS for p in pairs) or args.count <= 0 or (args.perf is not None and args.perf <= 0):
        parser.error('use py,rs pairs and positive counts')
    if args.typed and args.mutants:
        parser.error('typed mode does not support flat store probes')
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
        variant = 'typed' if args.typed else 'base'
        h = WORK / variant / pair / 'harness'
        harnesses[pair] = h if args.no_build and h.exists() else build(pair, variant, typed=args.typed)
        print(f'[{PAIRS[pair]}] built', flush=True)
    if args.hdiff:
        corpus = generated + fixtures + crafted() + crafted(big=True)
        q7 = json.loads((CORPUS / 'q7_cases.json').read_text())
        checks = []
        for pair, h in harnesses.items():
            print(f'[{PAIRS[pair]}] ', end='')
            checks.append(hdiff(h, q7))
            checks.append(hdiff(h, corpus, legacy=True, label='legacy'))
            checks.append(hdiff(h, lifecycle_cases(), label='lifecycle'))
            checks.append(audit_crafted(h))
        return 0 if all(checks) else 1
    if args.cleanup_checks:
        return 0 if cleanup_checks(harnesses) else 1
    if args.perf:
        perf(harnesses[pairs[-1]], args.perf)
        print('item size Pruv-ns/op hand-ns/op Pruv-alloc/op hand-alloc/op checksum')
        for item, size, p, h, pa, ha, checksum in lifecycle_perf(harnesses[pairs[-1]], args.perf):
            print(f'{item} {size} {p:.1f} {h:.1f} {pa:.3f} {ha:.3f} {checksum}', flush=True)
        return 0
    good = True
    for pair, h in harnesses.items():
        problems = fixed_checks(h, typed=args.typed)
        for msg in problems[:5]:
            print(f'  [{PAIRS[pair]}] {msg}', flush=True)
        good &= not problems
    suites = {'Q7': q_gen.gen(args.seed, ['Q7'], count=args.count), 'Q7P': generated, 'L': fixtures,
              'crafted': crafted(), 'lifecycle': lifecycle_cases()}
    expected = {name: observations(cases, allow_prefix=name != 'Q7') for name, cases in suites.items()}
    for pair, h in harnesses.items():
        print(f'[{PAIRS[pair]}] ', end='')
        good &= audit_crafted(h)
        repro = next((row for row in expected['Q7'] if row[0]['id'] == 'Q7-20261117-3'), None)
        if repro is not None:
            good &= save_snapshot_repro(h, repro)
        for name, rows in expected.items():
            counts = replay(h, rows)
            print(f'[{PAIRS[pair]}] {name}: ' + ' '.join(f'{k}={counts[k]}' for k in ('exact', 'prefix-ok', 'prefix-fail', 'fail')), flush=True)
            good &= not (counts['fail'] or counts['prefix-fail'])
            good &= counts['exact'] == len(rows)
    return 0 if good else 1


if __name__ == '__main__':
    sys.exit(main())
