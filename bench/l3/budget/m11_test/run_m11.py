#!/usr/bin/env python3
"""L3-10 M11 driver: work cards (bud_card.pruv, RequestBudgets.setCard / hasCard / current / isHalted) against the oracle.

  python3 run_m11.py [--pairs py,rs] [--count 300] [--seed 20261117] [--perf] [--no-build]

1. copies root.pruv + bud_card.pruv and the modules it uses into work/m11/src/
2. compiles with the Python compiler (py/rust) and/or the Rust compiler (rs/rust), then rustc -O harness.rs per pair
3. cases: q_gen.gen(seed, ['Q6'], count); expected: oracle.run_oracle(cases) (the pinned TS)
4. replays each case through the harness (stand-in session store in root.pruv) and compares in canonical JV space:
   every request value (budget wrappers {bref, v}: the bref must be the named session's budget, then v is compared),
   per-request trace counts of findSession / now / save / changed, and finalSessions (id + budget).
   Cases with a `halt` op are not-implemented (M11 has no halt): only the prefix before the first halt is replayed and
   reported as prefix-ok / prefix-fail; they never count as passes.
5. --perf: item C (diagnostic, Pruv vs hand only) = B without the stand-in store and out words; item A (validate+build, U = 100/300/1000) and item B (n = 100/500/1000 successive setCard) for Pruv, hand.rs
   and the pinned TS (perf_ts.mjs); medians of >= 5 alternating runs.
Report: work/m11/report.json. Exit 1 on any fail, 2 on a build or driver failure.
"""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import os
import shutil
import statistics
import struct
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
BUDGET = HERE.parent
REPO = HERE.parents[3]
WORK = BUDGET / 'work' / 'm11'
LIB = REPO / 'bench' / 'l3' / 'lib'
sys.path.insert(0, str(BUDGET))
from common import NODE, TS_ENV              # noqa: E402
import q_gen                                 # noqa: E402
import oracle                                # noqa: E402
from compare import jv_decode, jv_encode, UNDEF   # noqa: E402

RS_DEFAULT = '/Users/spica4345/Projects/pruv/pruvc-rs/target/release/pruvc'
PY_DEFAULT = '/Users/spica4345/Projects/pruv/.venv/bin/python'
MODULES = ('bud_card.pruv', 'bud_keys.pruv', 'bud_num.pruv', 'bud_text.pruv', 'bud_date.pruv', 'bud_store.pruv', 'bud_map.pruv', 'bud_world.pruv')
PAIRS = {'py': 'py/rust', 'rs': 'rs/rust'}
SAFE = 2 ** 53 - 1
COUNTED = ('findSession', 'now', 'save', 'changed')
QUERY = {'current': 1, 'isHalted': 2, 'hasCard': 3}

# Known differences: (case id, request id) -> written reason. Empty: none found.
KNOWN = {}


def tool_env():
    env = dict(os.environ)
    env['PATH'] = str(Path.home() / '.cargo' / 'bin') + os.pathsep + env.get('PATH', '')
    return env


def sh(cmd, data=None, cwd=None, env=None):
    return subprocess.run([str(c) for c in cmd], input=data, capture_output=True, text=True, env=env or tool_env(), cwd=cwd)


# ---- build -------------------------------------------------------------------------------------------------------------
def prepare_sources():
    src = WORK / 'src'
    src.mkdir(parents=True, exist_ok=True)
    shutil.copy(HERE / 'root.pruv', src / 'root.pruv')
    for m in MODULES:
        shutil.copy(BUDGET / m, src / m)
    return src / 'root.pruv'


def build_pair(key, root):
    out = WORK / key
    out.mkdir(parents=True, exist_ok=True)
    sol = out / 'sol.rs'
    if key == 'rs':
        cmd = [os.environ.get('PRUVC_RS') or RS_DEFAULT, root, '-o', sol, '--lib', LIB]
    else:
        cmd = [os.environ.get('PRUVC_PY_PYTHON') or PY_DEFAULT, REPO / 'pruvc' / 'pruvc.py', root, '-o', sol,
               '--backend', 'rust', '--lib', LIB]
    r = sh(cmd, cwd=REPO)
    if r.returncode != 0 or not sol.exists():
        sys.stderr.write(f'[{key}] compile failed:\n{r.stdout[-3000:]}\n{r.stderr[-3000:]}\n')
        sys.exit(2)
    main = (HERE / 'harness.rs').read_text().replace('@SOL@', str(sol)).replace('@HAND@', str(HERE / 'hand.rs'))
    (out / 'main.rs').write_text(main)
    r = sh(['rustc', '-O', '--edition', '2021', '-A', 'warnings', out / 'main.rs', '-o', out / 'harness'])
    if r.returncode != 0:
        sys.stderr.write(f'[{key}] rustc failed:\n{r.stderr[-3000:]}\n')
        sys.exit(2)
    return out / 'harness'


# ---- V words ---------------------------------------------------------------------------------------------------------------
def units(s):
    raw = s.encode('utf-16-le', errors='surrogatepass')
    return [struct.unpack_from('<H', raw, i)[0] for i in range(0, len(raw), 2)]


def from_units(u):
    return b''.join(struct.pack('<H', x) for x in u).decode('utf-16-le', errors='surrogatepass')


def bits(f):
    return struct.unpack('<q', struct.pack('<d', f))[0]


def to_v(x, o):
    if x is UNDEF:
        o.append(1)
    elif x is None:
        o.append(2)
    elif x is False:
        o.append(3)
    elif x is True:
        o.append(4)
    elif isinstance(x, int):
        if abs(x) <= SAFE:
            o += [5, x]
        else:
            o += [7, bits(float(x))]
    elif isinstance(x, float):
        neg0 = x == 0 and str(x).startswith('-')
        if x == x and x.is_integer() and abs(x) <= SAFE and not neg0:
            o += [5, int(x)]
        else:
            o += [7, bits(x)]
    elif isinstance(x, str):
        u = units(x)
        o += [6, len(u)] + u
    elif isinstance(x, list):
        o += [8, len(x)]
        for e in x:
            to_v(e, o)
    elif isinstance(x, dict):
        o += [9, len(x)]
        for k, e in x.items():
            u = units(k)
            o += [len(u)] + u
            to_v(e, o)
    else:
        raise TypeError(type(x))
    return o


def from_v(w, p=0):
    t = w[p]
    if t == 1:
        return UNDEF, p + 1
    if t in (2, 3, 4):
        return (None, False, True)[t - 2], p + 1
    if t == 5:
        return w[p + 1], p + 2
    if t == 7:
        return struct.unpack('<d', struct.pack('<q', w[p + 1]))[0], p + 2
    if t == 6:
        n = w[p + 1]
        return from_units(w[p + 2:p + 2 + n]), p + 2 + n
    if t == 8:
        n, q, out = w[p + 1], p + 2, []
        for _ in range(n):
            e, q = from_v(w, q)
            out.append(e)
        return out, q
    if t == 9:
        n, q, out = w[p + 1], p + 2, {}
        for _ in range(n):
            kn = w[q]
            k = from_units(w[q + 1:q + 1 + kn])
            e, q = from_v(w, q + 1 + kn)
            out[k] = e
        return out, q
    raise ValueError(f'bad tag {t} at {p}')


def V(x):
    return to_v(x, [])


def wline(*parts):
    return ' | '.join(' '.join(str(x) for x in p) for p in parts)


# ---- expected side -------------------------------------------------------------------------------------------------------
def strip_brefs(x, brefs):
    """Replace every {bref, v} wrapper by v; collect the brefs seen."""
    if isinstance(x, dict):
        if set(x) == {'bref', 'v'}:
            brefs.append(x['bref'])
            return strip_brefs(x['v'], brefs)
        return {k: strip_brefs(e, brefs) for k, e in x.items()}
    if isinstance(x, list):
        return [strip_brefs(e, brefs) for e in x]
    return x


def canon(x):
    return json.dumps(jv_encode(x), sort_keys=False, ensure_ascii=False)


# ---- one case ----------------------------------------------------------------------------------------------------------
def case_lines(case):
    """Harness input lines for one case, and per call the bookkeeping to read the answers back."""
    world = case['world']
    lines = ['init']
    for s in world.get('sessions', []):
        sess = jv_decode(s)
        b = sess.get('budget', UNDEF)
        lines.append(wline(units(sess['id']), [] if b is UNDEF else V(b)))
        lines[-1] = 'load ' + lines[-1]
    now = world.get('clockStart', 1700000000000)
    calls = []
    halted = False
    for ev in case['script']:
        if ev['ev'] == 'advance':
            now += ev['ms']
            continue
        if ev['ev'] != 'call':
            continue
        if ev['op'] == 'halt':
            halted = True
            break
        args = [jv_decode(a) for a in ev['args']]
        if ev['op'] == 'setCard':
            pad = args + [None] * 4
            lang = [] if len(args) < 3 else V(args[2])
            by = [] if len(args) < 4 else V(args[3])
            lines.append('set ' + wline([now], V(args[0]), V(args[1]), lang, by))
        elif ev['op'] in QUERY:
            lines.append('q ' + wline([QUERY[ev['op']]], V(args[0])))
        else:
            calls.append({'req': ev['req'], 'op': ev['op'], 'unsupported': True})
            continue
        calls.append({'req': ev['req'], 'op': ev['op'], 'ref': args[0]})
    lines.append('dump')
    return lines, calls, halted


def parse_ok(line):
    if not line.startswith('ok'):
        raise ValueError(line)
    return [int(x) for x in line.split()[1:]]


def judge(case, exp, answers, calls, halted):
    """Returns (class, details)."""
    problems = []
    final_brefs = {}
    for fs in exp.get('finalSessions', []):
        v = fs['v']
        sess = jv_decode(v)
        bud = sess.get('budget') if isinstance(sess, dict) else None
        if isinstance(bud, dict) and 'bref' in bud:
            final_brefs[sess['id']] = bud['bref']
    reqs = {r['id']: r for r in exp['requests']}
    counts = {}
    for t in exp['trace']:
        if t['c'] in COUNTED:
            counts.setdefault(t.get('r'), {k: 0 for k in COUNTED})[t['c']] += 1
    it = iter(answers)
    for c in calls:
        if c.get('unsupported'):
            problems.append(f"{c['req']}: op {c['op']} not supported")
            continue
        line = next(it)
        er = reqs.get(c['req'])
        if er is None:
            problems.append(f"{c['req']}: no expected request")
            continue
        if er['terminal'] != 'ok':
            problems.append(f"{c['req']}: expected terminal {er['terminal']}")
        try:
            w = parse_ok(line)
        except ValueError:
            problems.append(f"{c['req']}: harness said {line[:200]}")
            continue
        if c['op'] == 'setCard':
            used_now, saved, changed, finds = w[0], w[1], w[2], w[3]
            got, end = from_v(w, 4)
        else:
            finds, used_now, saved, changed = w[0], 0, 0, 0
            got, end = from_v(w, 1)
        brefs = []
        want = strip_brefs(jv_decode(er['value']), brefs)
        ref = c['ref']
        sid = ref.get('sessionId') if isinstance(ref, dict) else None
        for b in brefs:
            if final_brefs.get(sid) != b:
                problems.append(f"{c['req']}: bref {b} is not the budget of session {sid!r}")
        if canon(got) != canon(want):
            problems.append(f"{c['req']}: value\n   want {canon(want)[:400]}\n   got  {canon(got)[:400]}")
        ec = counts.get(c['req'], {k: 0 for k in COUNTED})
        gc = {'findSession': finds, 'now': used_now, 'save': saved, 'changed': changed}
        if ec != gc:
            problems.append(f"{c['req']}: counts want {ec} got {gc}")
    dump = parse_ok(next(it))
    if not halted:
        n, p, got_s = dump[0], 1, []
        for _ in range(n):
            kn = dump[p]
            sid = from_units(dump[p + 1:p + 1 + kn])
            p += 1 + kn
            bn = dump[p]
            blob = dump[p + 1:p + 1 + bn]
            p += 1 + bn
            s = {'id': sid}
            if bn:
                s['budget'] = from_v(blob)[0]
            got_s.append(s)
        want_s = [strip_brefs(jv_decode(fs['v']), []) for fs in exp.get('finalSessions', [])]
        if canon(got_s) != canon(want_s):
            problems.append(f"finalSessions\n   want {canon(want_s)[:400]}\n   got  {canon(got_s)[:400]}")
    if halted:
        return ('prefix-fail' if problems else 'prefix-ok'), problems
    if problems:
        if all(any(p.startswith(f'{r}:') for (cid, r) in KNOWN if cid == case['id']) for p in problems):
            return 'known-difference', problems
        return 'fail', problems
    return 'exact', []


def run_pair(harness, cases, obs):
    allin, meta = [], []
    for case in cases:
        lines, calls, halted = case_lines(case)
        allin += lines
        meta.append((len(lines), calls, halted))
    r = sh([harness], data='\n'.join(allin) + '\n')
    if r.returncode != 0:
        sys.stderr.write(r.stderr[-2000:])
        sys.exit(2)
    outs = r.stdout.splitlines()
    res, pos = [], 0
    for case, exp, (nl, calls, halted) in zip(cases, obs, meta):
        chunk = outs[pos:pos + nl]
        pos += nl
        answers = chunk[1 + len(case['world'].get('sessions', [])):]
        cls, det = judge(case, exp, answers, calls, halted)
        res.append({'id': case['id'], 'class': cls, 'problems': det, 'raw': chunk})
    return res


# ---- perf --------------------------------------------------------------------------------------------------------------
def ns_of(line):
    p = line.split()
    if p[0] != 'ns':
        raise RuntimeError(line)
    return float(p[1])


def perf(harness, runs):
    node = [NODE, '--experimental-strip-types', '--no-warnings', '--import', (BUDGET / 'reference-hooks.mjs').as_uri(),
            HERE / 'perf_ts.mjs']
    # item A at size 1000 is the cheap error path (~20 ns): enough reps that one run is not a few microseconds of cold start
    items = [('A', u, max(2000, 2000000 // u)) for u in (100, 300, 1000)] + [('B', n, max(5, 20000 // n)) for n in (100, 500, 1000)] + \
        [('C', n, max(5, 20000 // n)) for n in (100, 1000)]
    # the hand baseline must write the same result words as the Pruv entry (same work), incl. every optional field
    chk = sh([harness], data='handcheck\n').stdout.strip()
    print(f'  handcheck: {chk}')
    if chk != 'same':
        raise RuntimeError('hand.rs differs from the Pruv entry: ' + chk)
    rows = []
    for item, size, reps in items:
        pv, hv, tv = [], [], []
        for r in range(runs):
            # alternate which side runs first in the process, so neither always pays the cold start
            if r % 2 == 0:
                out = sh([harness], data=f'perf{item} {size} {reps}\nhand{item} {size} {reps}\n').stdout.splitlines()
                pv.append(ns_of(out[0]))
                hv.append(ns_of(out[1]))
            else:
                out = sh([harness], data=f'hand{item} {size} {reps}\nperf{item} {size} {reps}\n').stdout.splitlines()
                hv.append(ns_of(out[0]))
                pv.append(ns_of(out[1]))
            if item != 'C':
                r = sh(node + [item, size, reps], cwd=BUDGET, env=TS_ENV())
                tv.append(ns_of(r.stdout.strip() or 'TS failed: ' + r.stderr[-500:]))
        p, h = statistics.median(pv), statistics.median(hv)
        t = statistics.median(tv) if tv else float('nan')
        rows.append({'item': item, 'size': size, 'pruv_ns': p, 'hand_ns': h, 'ts_ns': t, 'pruv_hand': round(p / h, 2)})
        print(f'  {item} | {size:5} | {p:9.1f} | {h:9.1f} | {t:9.1f} | {p / h:.2f}')
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--pairs', default='py,rs')
    ap.add_argument('--count', type=int, default=300)
    ap.add_argument('--seed', type=int, default=q_gen.SEED if hasattr(q_gen, 'SEED') else 20261117)
    ap.add_argument('--perf', action='store_true')
    ap.add_argument('--runs', type=int, default=5)
    ap.add_argument('--no-build', action='store_true')
    a = ap.parse_args()
    WORK.mkdir(parents=True, exist_ok=True)
    pairs = [p.strip() for p in a.pairs.split(',') if p.strip()]
    root = prepare_sources()
    harness = {}
    for k in pairs:
        h = WORK / k / 'harness'
        harness[k] = h if (a.no_build and h.exists()) else build_pair(k, root)
    cases = q_gen.gen(a.seed, ['Q6'], count=a.count)
    obs = oracle.run_oracle(cases)
    report = {'seed': a.seed, 'count': len(cases), 'pairs': {}}
    raws = {}
    bad = False
    for k in pairs:
        res = run_pair(harness[k], cases, obs)
        tally = {}
        for r in res:
            tally[r['class']] = tally.get(r['class'], 0) + 1
        fails = [r for r in res if r['class'] in ('fail', 'prefix-fail')]
        bad = bad or bool(fails)
        print(f'[{PAIRS[k]}] {tally}')
        for r in fails[:8]:
            print(f"  {r['class']} {r['id']}:")
            for p in r['problems'][:4]:
                print('   ', p)
        report['pairs'][k] = {'tally': tally,
                              'cases': [{'id': r['id'], 'class': r['class'], 'problems': r['problems']} for r in res]}
        raws[k] = [r['raw'] for r in res]
    if len(raws) > 1:
        same = all(v == next(iter(raws.values())) for v in raws.values())
        report['pairs_identical'] = same
        print(f'pairs identical: {same}')
        bad = bad or not same
    report['known_differences'] = {f'{c}/{r}': why for (c, r), why in KNOWN.items()}
    if a.perf:
        print('  item | size | Pruv ns | hand Rust ns | TS ns | Pruv/hand')
        report['perf'] = perf(harness[pairs[-1]], a.runs)
    (WORK / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False))
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
