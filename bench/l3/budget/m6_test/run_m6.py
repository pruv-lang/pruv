#!/usr/bin/env python3
"""L3-10 M6 driver: request-budget number/text/date helpers (bud_num, bud_text, bud_date) against the pinned TypeScript, per case.

  python3 run_m6.py [--pairs py,rs] [--count 300] [--seed 20261117] [--perf] [--no-build]

1. copies root.pruv + ../bud_num.pruv ../bud_text.pruv ../bud_date.pruv into work/m6/src/
2. compiles with the Python compiler (py/rust) and/or the Rust compiler (rs/rust), then rustc -O harness.rs per pair
3. generates exactly --count deterministic cases (xorshift32), asks node_ref.mjs for the expected words, runs each harness, compares
4. --perf: compare Python- and Rust-compiled helpers with hand Rust (hand.rs, linked into the same harness) at 100, 500 and 1000 calls
Report: work/m6/report.json. Exit 1 on any mismatch, 2 on a build or driver failure.
"""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import os
import re
import shutil
import struct
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
BUDGET = HERE.parent
REPO = HERE.parents[3]                       # m6_test -> budget -> l3 -> bench -> repo root
WORK = BUDGET / 'work' / 'm6'
LIB = REPO / 'bench' / 'l3' / 'lib'
sys.path.insert(0, str(BUDGET))
from common import NODE, TS_ENV              # noqa: E402

RS_DEFAULT = '/Users/spica4345/Projects/pruv/pruvc-rs/target/release/pruvc'
MODULES = ('bud_num.pruv', 'bud_text.pruv', 'bud_date.pruv')
PAIRS = {'py': 'py/rust', 'rs': 'rs/rust'}
SAFE = 2 ** 53 - 1
DMAX = 8640000000000000
M32 = 0xFFFFFFFF
WEIGHTS = [('class', 50), ('int', 45), ('floor', 40), ('iso', 45), ('arith', 30), ('text', 60), ('trim', 30)]
PERF_OPS = [
    ('p_iso', [1700000000000]),
    ('p_ref', [2, 0, 12, 99999, 0]),
    ('p_int', [1234567890123]),
    ('p_amount', [0, 0, -60001, 99999, 0]),
    ('p_card_error', [2, 0]),
]
PERF_SIZES = (100, 500, 1000)
PERF_WARMUP = 3
PERF_REPS = 9


# ---------------------------------------------------------------- random stream (xorshift32, rejection sampling like q_gen.Rng)
class Rng:
    def __init__(self, seed):
        self.s = (seed & M32) or 1

    def next32(self):
        x = self.s
        x ^= (x << 13) & M32
        x ^= x >> 17
        x ^= (x << 5) & M32
        self.s = x
        return x

    def below(self, n):
        limit = (1 << 32) // n * n
        while True:
            x = self.next32()
            if x < limit:
                return x % n

    def pick(self, seq):
        return seq[self.below(len(seq))]

    def shuffle(self, items):
        for i in range(len(items) - 1, 0, -1):
            j = self.below(i + 1)
            items[i], items[j] = items[j], items[i]
        return items


def cat_rng(seed, idx):
    r = Rng((seed * 2654435761 + (idx + 1) * 40503) & M32)
    for _ in range(4):
        r.next32()
    return r


def rsafe(r):
    """A random safe integer with a random magnitude (0..53 bits) and sign."""
    mag = r.below(54)
    v = (((r.next32() << 32) | r.next32()) & ((1 << mag) - 1)) if mag else 0
    v = min(v, SAFE)
    return -v if r.below(2) else v


def interleave(lists, quota):
    """Round-robin over the candidate lists until quota cases are taken."""
    lists = [list(x) for x in lists if x]
    out, j = [], 0
    while len(out) < quota:
        if not lists:
            raise RuntimeError(f'not enough candidate cases for quota {quota}')
        j %= len(lists)
        out.append(lists[j].pop(0))
        if lists[j]:
            j += 1
        else:
            del lists[j]
    return out


def must_first(must, lists, quota):
    """Take every required case first (as many as fit), then fill the rest round-robin from lists."""
    head = list(must)[:quota]
    return head + (interleave(lists, quota - len(head)) if quota > len(head) else [])


# ---------------------------------------------------------------- case construction
def mk(kind, label, op, words, req):
    r = dict(req)
    r['op'] = op
    return {'kind': kind, 'label': label, 'line': ' '.join([op] + [str(w) for w in words]), 'req': r}


def units(s):
    raw = s.encode('utf-16-le', 'surrogatepass')
    return [raw[i] | (raw[i + 1] << 8) for i in range(0, len(raw), 2)]


def sw(us):
    return [len(us)] + list(us)


def hexof(x):
    return struct.pack('>d', x).hex()


def signed(h):
    v = int(h, 16)
    return v - (1 << 64) if v >= (1 << 63) else v


def collect_nums(obj, out):
    if isinstance(obj, dict):
        if set(obj) == {'num'} and isinstance(obj['num'], str):
            if re.fullmatch(r'[0-9a-f]{16}', obj['num']):
                out.append(obj['num'])
            return
        for v in obj.values():
            collect_nums(v, out)
    elif isinstance(obj, list):
        for v in obj:
            collect_nums(v, out)


def number_pools(seed):
    """Raw {num} hex values: from fixtures K and N, and from q_gen cases of the families Q6 and Q8."""
    fx, qg = [], []
    for letter in 'KN':
        with open(BUDGET / 'fixtures' / f'{letter}.json', encoding='utf-8') as f:
            collect_nums(json.load(f), fx)
    import q_gen
    for case in q_gen.gen(seed, ['Q6', 'Q8'], 40):
        collect_nums(case, qg)
    dedup = lambda xs: list(dict.fromkeys(xs))
    return dedup(fx), dedup(qg)


def cases_class(r, quota, pools):
    edge = [0.0, -0.0, 1.0, 0.5, -1.0, 2.0 ** 53, 2.0 ** 53 + 2, 1e300, 5e-324, float('nan'), float('inf'), float('-inf'), 1.7976931348623157e308,
            -1e300, -0.5, 2.0 ** 31, -(2.0 ** 53)]
    edge_h = [hexof(x) for x in edge] + ['7ff8000000000001', 'fff8000000000000', '7ff0000000000001']   # NaN payloads / signaling NaN
    rnd = []
    for _ in range(quota):
        k = r.below(4)
        if k == 0:
            rnd.append('%08x%08x' % (r.next32(), r.next32()))
        elif k == 1:
            rnd.append(hexof(float(rsafe(r))))
        elif k == 2:
            rnd.append(hexof(rsafe(r) / (1 + r.below(1000))))
        else:
            rnd.append(hexof(float(rsafe(r)) * 10.0 ** r.below(300)))
    fx = r.shuffle(list(pools[0]))
    qg = r.shuffle(list(pools[1]))
    mkc = lambda h: mk('class', 'node', 'cls', [signed(h)], {'hex': h})
    return interleave([[mkc(h) for h in edge_h], [mkc(h) for h in fx], [mkc(h) for h in qg], [mkc(h) for h in rnd]], quota)


def cases_int(r, quota):
    edge = [0, 1, -1, SAFE, -SAFE, SAFE - 1, -(SAFE - 1), 2 ** 31, 2 ** 31 - 1, -(2 ** 31), 2 ** 32, 2 ** 52]
    for k in range(1, 16):
        edge += [10 ** k, 10 ** k - 1, 10 ** k + 1, -(10 ** k)]
    rnd = [rsafe(r) for _ in range(quota)]
    mkc = lambda x: mk('int', 'node', 'int', [x], {'x': str(x)})
    return interleave([[mkc(x) for x in edge], [mkc(x) for x in rnd]], quota)


def cases_floor(r, quota):
    mins = []
    for k in (0, 1, 2, 3, -1, -2, -3, 1000, -1000, 150119987578, 150119987579, -150119987579):
        for off in (-1, 0, 1):
            ms = k * 60000 + off
            if abs(ms) <= SAFE:
                mins.append(ms)
    mins += [SAFE, -SAFE, SAFE - 1, 1, -1, 59999, -59999, 60000, -60000, 119999, -119999]
    fd = [(0, 1), (-1, 1), (7, 2), (-7, 2), (SAFE, 2), (-SAFE, 2), (SAFE, SAFE), (-SAFE, SAFE), (-1, SAFE), (59999, 60000), (-59999, 60000),
          (60000, 60000), (-60001, 60000), (SAFE, 1), (-SAFE, 1)]
    rmin, rfd = [], []
    for _ in range(quota):
        if r.below(2):
            rmin.append(rsafe(r))
        else:
            k = (rsafe(r) % 300000000000) - 150000000000
            rmin.append(max(-SAFE, min(SAFE, k * 60000 + r.pick([-1, 0, 1, 59999]))))
        d = r.pick([1, 2, 7, 1000, 60000, 86400000, 2 ** 40, SAFE]) if r.below(2) else max(1, abs(rsafe(r)))
        rfd.append((rsafe(r), d))
    mm = lambda ms: mk('floor.min', 'node', 'min', [ms], {'ms': str(ms)})
    mf = lambda t: mk('floor.fdiv', 'node', 'fdv', [t[0], t[1]], {'a': str(t[0]), 'd': str(t[1])})
    return interleave([[mm(x) for x in mins], [mf(t) for t in fd], [mm(x) for x in rmin], [mf(t) for t in rfd]], quota)


def cases_iso(r, quota):
    edge = [0, -1, 1, DMAX, -DMAX, DMAX - 1, -DMAX + 1,
            -62167219200000, -62167219200001, -62167219199999,    # year 0 start, year -1 last ms, year 0 start + 1
            253402300799999, 253402300800000, 253402300799998,    # year 9999 last ms, year 10000 start
            -62198755200000, -62198755200001,                     # year -1 start, year -2 last ms
            -30610224000000, 951782400000, 1709164800000, 1700000000000, 86399999, -86400001, 4102444800000]
    rnd = []
    for _ in range(quota):
        v = rsafe(r) if r.below(2) else rsafe(r) % 4102444800000
        rnd.append(max(-DMAX, min(DMAX, v)))
    dok = [0, DMAX, -DMAX, DMAX + 1, -DMAX - 1, SAFE, -SAFE, DMAX - 1, -DMAX + 1, 1700000000000]
    mi = lambda ms: mk('iso', 'node', 'iso', [ms], {'ms': str(ms)})
    md = lambda ms: mk('iso.dok', 'node', 'dok', [ms], {'ms': str(ms)})
    return interleave([[mi(x) for x in edge], [mi(x) for x in rnd], [md(x) for x in dok]], quota)


def cases_arith(r, quota):
    a = 94906265
    add = [(SAFE, 0), (SAFE, 1), (SAFE - 1, 1), (-SAFE, -1), (-SAFE, 0), (SAFE, -SAFE), (1, 2), (SAFE, SAFE), (-SAFE, -SAFE), (SAFE - 5, 5), (SAFE - 5, 6)]
    sub = [(0, SAFE), (-SAFE, 1), (SAFE, -1), (SAFE, SAFE), (-SAFE, SAFE), (0, -SAFE), (5, 7), (-SAFE + 4, 4), (-SAFE + 4, 5), (SAFE - 2, -2), (SAFE - 2, -3)]
    mul = [(a, SAFE // a), (a, SAFE // a + 1), (2 ** 26, 2 ** 27), (2 ** 26, 2 ** 27 - 1), (SAFE, 1), (SAFE, -1), (-SAFE, 1), (SAFE, 2), (SAFE, 0),
           (0, -SAFE), (a, a), (a + 1, a + 1), (-(a + 1), a + 1), (3, 3002399751580330), (3, 3002399751580331), (-3, 3002399751580331)]
    mx = [(1, 2), (-5, -7), (SAFE, -SAFE), (0, 0)]
    rnd = []
    for _ in range(quota):
        o = r.below(3)
        x, y = (rsafe(r), rsafe(r)) if r.below(2) else (rsafe(r) % (2 ** 27), rsafe(r) % (2 ** 27))
        rnd.append((o, x, y))
    ma = lambda o, x, y, kind: mk(kind, 'node', 'ari', [o, x, y], {'o': o, 'a': str(x), 'b': str(y)})
    lists = [[ma(0, x, y, 'arith.add') for x, y in add], [ma(1, x, y, 'arith.sub') for x, y in sub], [ma(2, x, y, 'arith.mul') for x, y in mul],
             [ma(3, x, y, 'arith.max') for x, y in mx], [ma(o, x, y, ['arith.add', 'arith.sub', 'arith.mul'][o]) for o, x, y in rnd]]
    safe_vals = [SAFE, SAFE + 1, -SAFE, -SAFE - 1, 2 ** 62, -(2 ** 63), 2 ** 63 - 1, 0, 1, -1]
    lists.append([mk('arith.issafe', 'node', 'sfe', [v], {'v': str(v)}) for v in safe_vals])
    ok_vals = [0, 1, -1, SAFE, -SAFE, 5, -5, 2 ** 31]
    lists.append([mk('arith.okint', 'node', 'oki', [v], {'v': str(v)}) for v in ok_vals])
    return interleave(lists, quota)


def time_ms(reason, used, rem):
    # Pruv side receives activeMs for the time reason (the helper floors it); clamped to the safe domain
    if reason != 0:
        return used
    return max(-SAFE, min(SAFE, used * 60000 + rem))


def amount_args(r, reason):
    """used/limit like q_gen Q8: 0, equal to the limit, just around it, large. Time `used` is minutes (+ a remainder in ms for the TS side)."""
    limit = r.pick([0, 1, 2, 3, 24, 120, 99999, 10 ** 9, SAFE]) if r.below(4) else abs(rsafe(r))
    used = [0, limit, max(limit - 1, 0), limit + 1, 1, abs(rsafe(r)), -1, 10 ** 9][r.below(8)]
    used = max(-SAFE, min(SAFE, used))
    rem = 0
    if reason == 0:
        used = max(-10 ** 9, min(10 ** 9, used))
        rem = r.below(60000)
    return used, limit, rem


def cases_text(r, quota):
    def amt(kind, op, n, fixed=()):
        out = []
        for reason, unl, used, limit, rem, lang in list(fixed):
            out.append((reason, unl, used, limit, rem, lang))
        while len(out) < n:
            reason, unl, lang = r.below(5), 1 if r.below(3) == 0 else 0, r.below(2)
            used, limit, rem = amount_args(r, reason)
            out.append((reason, unl, used, limit, rem, lang))
        return [mk(f'text.{kind}', 'node', op, [c[0], c[1], time_ms(c[0], c[2], c[4]), c[3], c[5]],
                   {'reason': c[0], 'unl': c[1], 'used': str(c[2]), 'limit': str(c[3]), 'rem': c[4], 'lang': c[5],
                    'ms': str(time_ms(c[0], c[2], c[4]))}) for c in out]
    special = [(0, 0, 150119987579, SAFE, 991, 0), (0, 0, 0, 0, 0, 1), (2, 1, 3, 3, 0, 0), (2, 1, 3, 3, 0, 1), (4, 0, SAFE, SAFE, 0, 1)]
    refusal = amt('refusal', 'ref', quota, special)
    cont = amt('continue', 'con', quota, special)
    amount = amt('amount', 'amt', quota, special)
    limit_t = []
    for i in range(quota):
        unl, lang = (1 if i % 3 == 0 else 0), r.below(2)
        limit = [0, 1, SAFE, 120, -1][i] if i < 5 else (abs(rsafe(r)) if r.below(2) else r.pick([0, 1, 3, 120, 99999]))
        limit_t.append(mk('text.limit', 'node', 'lmt', [unl, limit, lang], {'unl': unl, 'limit': str(limit), 'lang': lang}))
    wire = []
    for i in range(quota):
        reason = r.below(5)
        used, limit, rem = amount_args(r, reason)
        wire.append(mk('text.wire', 'node', 'wir', [reason, time_ms(reason, used, rem), limit],
                       {'reason': reason, 'used': str(used), 'limit': str(limit), 'rem': rem, 'ms': str(time_ms(reason, used, rem))}))
    lang_strs = [None, 'ko', 'en', 'ja', 'zh', '', 'EN', 'ko ', 'en-US', 'KO', 'jp']
    lang = [mk('text.lang_of', 'node', 'lng', [0 if s is None else 1] + sw([] if s is None else units(s)),
               {'present': 0 if s is None else 1, 'u': [] if s is None else units(s)}) for s in lang_strs]
    lex = [mk('text.lang_excl', 'node', 'lex', sw(units(s)), {'u': units(s)}) for s in ['ko', 'en', 'ja', '', 'constructor', '__proto__', 'toString', 'valueOf', 'hasOwnProperty', 'toStrin']]
    cerr = [mk('text.card_error', 'node', 'cer', [k, lg], {'kind': k, 'lang': lg}) for k in range(7) for lg in (0, 1)]
    cmi = [mk('text.card_missing', 'node', 'cmi', [lg], {'lang': lg}) for lg in (0, 1)]
    cnt = [mk('text.counter', 'node', 'cnt', [k, lg], {'reason': k, 'lang': lg}) for k in range(5) for lg in (0, 1)]
    nol = [mk('text.no_limit', 'node', 'nol', [lg], {'lang': lg}) for lg in (0, 1)]
    # every card-error kind, card_missing and no_limit in both languages are required; the rest is interleaved
    return must_first(cerr + cmi + nol, [refusal, cont, limit_t, amount, wire, lang, lex, cnt], quota)


WS = ['\t', '\n', '\x0b', '\x0c', '\r', ' ', '\xa0', ' ', ' ', ' ', ' ', ' ', ' ', ' ', '　', '﻿']
NOT_WS = ['\x85', '​', '᠎']
EM = '\U0001F600'


def cases_trim(r, quota):
    fixed = ['', ' ', 'a', 'a' * 300, 'a' * 301, ' ' + 'a' * 300 + ' ', ' ' * 10, '﻿' + 'a' * 300 + '　', ' a ', '\ud800', 'a\ud800',
             '\udc00a', EM * 150, EM * 151, 'a' * 298 + EM, 'a' * 299 + EM, '\x85', '​', '᠎', 'x' * 300 + '\x85', '﻿', '　' * 301,
             'a' * 150 + ' ' + 'a' * 150, ' ' + EM + ' ']
    rnd = []
    for _ in range(quota):
        body_len = r.pick([0, 1, 298, 299, 300, 301, r.below(310)])
        body = ''.join(r.pick(['a', 'b', EM, '\ud800', '\udc00', '\x85', '​']) if r.below(12) == 0 else 'a' for _ in range(body_len))
        rnd.append(''.join(r.pick(WS) for _ in range(r.below(6))) + body + ''.join(r.pick(WS) for _ in range(r.below(6))))
    mkc = lambda s: mk('trim', 'node', 'len', sw(units(s)), {'u': units(s)})
    # the fixed boundary strings are required; random strings fill the rest
    return must_first([mkc(s) for s in fixed], [[mkc(s) for s in rnd]], quota)


def split_quota(count):
    total = sum(w for _, w in WEIGHTS)
    base = [count * w // total for _, w in WEIGHTS]
    order = sorted(range(len(WEIGHTS)), key=lambda i: (-(count * WEIGHTS[i][1] % total), i))
    for i in order[:count - sum(base)]:
        base[i] += 1
    return base


def build_cases(count, seed):
    quotas = split_quota(count)
    pools = number_pools(seed)
    fns = [lambda r, q: cases_class(r, q, pools), cases_int, cases_floor, cases_iso, cases_arith, cases_text, cases_trim]
    cases = []
    for idx, (fn, q) in enumerate(zip(fns, quotas)):
        cases += fn(cat_rng(seed, idx), q) if q else []
    assert len(cases) == count, (len(cases), count)
    return cases, dict(zip((n for n, _ in WEIGHTS), quotas))


# ---------------------------------------------------------------- tools
def tool_env():
    env = dict(os.environ)
    env['PATH'] = os.path.expanduser('~/.cargo/bin') + os.pathsep + env.get('PATH', '')
    return env


def sh(cmd, data=None, env=None, timeout=900):
    return subprocess.run([str(x) for x in cmd], input=data, text=True, capture_output=True, env=env or tool_env(), timeout=timeout)


def run_node(reqs):
    cmd = [NODE, '--experimental-strip-types', '--no-warnings', '--import', (BUDGET / 'reference-hooks.mjs').as_uri(), HERE / 'node_ref.mjs']
    p = sh(cmd, ''.join(json.dumps(r, separators=(',', ':')) + '\n' for r in reqs), env=TS_ENV())
    lines = [x for x in p.stdout.split('\n') if x.strip()]
    if p.returncode != 0 or len(lines) != len(reqs):
        sys.stderr.write(f'node_ref.mjs failed (rc {p.returncode}, {len(lines)} results for {len(reqs)} requests)\n{p.stderr}\n')
        raise SystemExit(2)
    return [json.loads(x) for x in lines]


def build_pair(key):
    """Compile root.pruv to Rust with one compiler and build the harness; stops the whole run on any error."""
    out = WORK / key
    out.mkdir(parents=True, exist_ok=True)
    sol = out / 'sol.rs'
    root = WORK / 'src' / 'root.pruv'
    if key == 'rs':
        cmd = [os.environ.get('PRUVC_RS') or RS_DEFAULT, root, '-o', sol, '--lib', LIB]
    else:
        cmd = [os.environ.get('PRUVC_PY_PYTHON', '/Users/spica4345/Projects/pruv/.venv/bin/python'), REPO / 'pruvc' / 'pruvc.py', root, '-o', sol, '--backend', 'rust', '--lib', LIB]
    print(f'[{PAIRS[key]}] compiling: {" ".join(str(c) for c in cmd)}', flush=True)
    p = sh(cmd)
    if p.returncode != 0:
        print(f'[{PAIRS[key]}] COMPILER ERROR (rc {p.returncode})\n{p.stdout}{p.stderr}')
        raise SystemExit(2)
    main_rs = out / 'main.rs'
    main_rs.write_text((HERE / 'harness.rs').read_text().replace('@SOL@', str(sol)).replace('@HAND@', str(HERE / 'hand.rs')))
    p = sh([os.environ.get('RUSTC', 'rustc'), '-O', '--edition', '2021', main_rs, '-o', out / 'harness'])
    if p.returncode != 0:
        print(f'[{PAIRS[key]}] RUSTC ERROR (rc {p.returncode})\n{p.stdout}{p.stderr}')
        raise SystemExit(2)
    print(f'[{PAIRS[key]}] built {out / "harness"}', flush=True)


def prepare_sources():
    src = WORK / 'src'
    if src.exists():
        shutil.rmtree(src)
    src.mkdir(parents=True)
    shutil.copy(HERE / 'root.pruv', src / 'root.pruv')
    for m in MODULES:
        if not (BUDGET / m).exists():
            print(f'missing module {BUDGET / m}')
            raise SystemExit(2)
        shutil.copy(BUDGET / m, src / m)


def run_harness(key, lines):
    exe = WORK / key / 'harness'
    if not exe.exists():
        print(f'[{PAIRS[key]}] no harness at {exe} (run without --no-build)')
        raise SystemExit(2)
    p = sh([exe], ''.join(x + '\n' for x in lines))
    out = p.stdout.split('\n')
    if out and out[-1] == '':
        out.pop()
    if p.returncode != 0 or len(out) != len(lines):
        print(f'[{PAIRS[key]}] harness failed (rc {p.returncode}, {len(out)} lines for {len(lines)} inputs)\n{p.stderr[-2000:]}')
        raise SystemExit(2)
    return out


# ---------------------------------------------------------------- comparison
def parse_words(line):
    if line.startswith('PANIC'):
        return line
    return [int(x) for x in line.split()]


def compare(key, cases, expected, got_lines):
    stats, mism = {}, []
    for c, e, g in zip(cases, expected, got_lines):
        k = stats.setdefault(c['kind'], [0, 0])
        k[0] += 1
        want = e.get('w')
        got = parse_words(g)
        if c['kind'] == 'floor.min' and 'exact' in e:
            want = [e['exact']]
        if want is not None and got == want:
            k[1] += 1
        else:
            mism.append({'kind': c['kind'], 'label': c['label'], 'line': c['line'][:300], 'expected': want if want is not None else 'ERR ' + e.get('err', '?'),
                         'got': got})
    return stats, mism


def clip(v):
    s = json.dumps(v, ensure_ascii=False)
    return s if len(s) <= 400 else s[:400] + '...'


# ---------------------------------------------------------------- perf
def perf_cell(exe, line, n, time_available):
    # One process per cell. The OS counters cover all runs, including warm-up.
    runs = PERF_WARMUP + PERF_REPS
    data = (line + '\n') * runs
    if time_available:
        proc = sh(['/usr/bin/time', '-l', exe], data)
    else:
        # A fresh wrapper gives us this one child's RSS when Darwin time cannot
        # read the system clock rate or when the host lacks Darwin time.
        proc = sh([sys.executable, '-c',
                   'import resource, subprocess, sys; '
                   'p = subprocess.run(sys.argv[1:]); '
                   'rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss; '
                   'print(rss * (1 if sys.platform == "darwin" else 1024), '
                   '" maximum resident set size", file=sys.stderr); '
                   'sys.exit(p.returncode)', exe], data)
    samples = re.findall(r'^ns (\d+) (-?\d+)$', proc.stdout, re.MULTILINE)
    if proc.returncode != 0 or len(samples) != runs:
        sys.exit(f'perf {exe.name} {line}: {proc.stdout}{proc.stderr}')

    def counter(name):
        match = re.search(rf'^\s*(\d+)\s+{name}\s*$', proc.stderr, re.MULTILINE)
        return int(match[1]) if match else None

    sums = {int(total) for _, total in samples}
    if len(sums) != 1:
        sys.exit(f'perf {exe.name} {line}: inconsistent checksums {sums}')
    return {
        'ns_per_call': min(int(ns) for ns, _ in samples[PERF_WARMUP:]) / n,
        'instructions': counter('instructions retired'),
        'rss_bytes': counter('maximum resident set size'),
        'sum': sums.pop(),
    }


def perf(keys):
    # The hand lane runs inside the first pair's harness (ops `hand_<op>`), so every lane is the same kind of process.
    lanes = {f'new-{key}': (WORK / key / 'harness', '') for key in keys}
    lanes['hand'] = (WORK / keys[0] / 'harness', 'hand_')
    time_available = False
    if sys.platform == 'darwin':
        probe = sh(['/usr/bin/time', '-l', '/usr/bin/true'])
        time_available = probe.returncode == 0
        if not time_available:
            print(f'perf counters: {probe.stderr.splitlines()[-1]}')
    jobs = [(op, args, n) for op, args in PERF_OPS for n in PERF_SIZES]
    oracle = run_node([{'op': 'perf', 'name': op, 'n': n, 'args': list(map(str, args)), 'reps': 3}
                       for op, args, n in jobs])

    print(f'\nperf: minimum of {PERF_REPS} in ns/call / process instructions / peak RSS bytes')
    print(f'{"helper":14}{"size":>5}' + ''.join(f'{lane:>30}' for lane in lanes) + '  slower than hand')
    report = []
    for (op, args, n), expected in zip(jobs, oracle):
        line = ' '.join(map(str, [op, n, *args]))
        cells = {lane: perf_cell(exe, prefix + line, n, time_available) for lane, (exe, prefix) in lanes.items()}
        if 'err' in expected or any(cell['sum'] != expected['sum'] for cell in cells.values()):
            sys.exit(f'perf {line}: checksum mismatch: Node {expected}, Rust {cells}')
        size = {100: 'S', 500: 'M'}.get(n, str(n))
        row = f'{op:14}{size:>5}'
        slower = []
        for lane, cell in cells.items():
            instructions = cell['instructions'] if cell['instructions'] is not None else 'n/a'
            rss = cell['rss_bytes'] if cell['rss_bytes'] is not None else 'n/a'
            formatted = f"{cell['ns_per_call']:.1f} / {instructions} / {rss}"
            row += f'{formatted:>30}'
            if lane != 'hand' and cell['ns_per_call'] > cells['hand']['ns_per_call']:
                slower.append(lane)
        print(row + '  ' + ', '.join(slower))
        report.append({'op': op, 'n': n, 'node_ns_per_call': expected['ns'] / n, **cells,
                       'slower_than_hand': slower})
    return report


# ---------------------------------------------------------------- main
def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--pairs', default='py,rs', help='comma list of py (py/rust) and rs (rs/rust)')
    ap.add_argument('--count', type=int, default=300)
    ap.add_argument('--seed', type=int, default=20261117)
    ap.add_argument('--perf', action='store_true')
    ap.add_argument('--no-build', action='store_true')
    a = ap.parse_args()
    # The generated writers must match the source template comments.
    g = subprocess.run([sys.executable, str(HERE / 'gen_text.py'), '--check'], text=True, capture_output=True)
    if g.returncode != 0:
        sys.exit(g.stderr.strip() or g.stdout.strip())
    keys = []
    for p in a.pairs.split(','):
        p = p.strip().split('/')[0]
        if p not in PAIRS:
            ap.error(f'unknown pair {p!r}')
        keys.append(p)
    WORK.mkdir(parents=True, exist_ok=True)

    if not a.no_build:
        prepare_sources()
        for k in keys:
            build_pair(k)

    cases, quotas = build_cases(a.count, a.seed)
    print(f'{len(cases)} cases (seed {a.seed}): ' + ' '.join(f'{k}={v}' for k, v in quotas.items()), flush=True)
    expected = run_node([c['req'] for c in cases])
    inexact = sum(1 for c, e in zip(cases, expected) if c['kind'] == 'floor.min' and 'exact' in e and e['exact'] != e['w'][0])
    labels = {}
    for c in cases:
        labels.setdefault(c['kind'], c['label'])

    report = {'seed': a.seed, 'count': a.count, 'quotas': quotas, 'labels': labels, 'ts_inexact_minutes': inexact, 'pairs': {}}
    failed = False
    for k in keys:
        stats, mism = compare(k, cases, expected, run_harness(k, [c['line'] for c in cases]))
        total = sum(v[0] for v in stats.values())
        exact = sum(v[1] for v in stats.values())
        print(f'\n== {PAIRS[k]}: total {total}, exact {exact}, mismatches {len(mism)}')
        for m in mism[:10]:
            print(f'  MISMATCH [{m["kind"]}/{m["label"]}] {m["line"]}\n    expected {clip(m["expected"])}\n    got      {clip(m["got"])}')
        print(f'  {"kind":18}{"source":>7}{"total":>7}{"exact":>7}')
        for kind in sorted(stats):
            print(f'  {kind:18}{labels[kind]:>7}{stats[kind][0]:>7}{stats[kind][1]:>7}')
        report['pairs'][PAIRS[k]] = {'total': total, 'exact': exact, 'mismatches': mism, 'kinds': {x: {'total': v[0], 'exact': v[1]} for x, v in stats.items()}}
        failed |= bool(mism)
    if inexact:
        print(f'\nnote: for {inexact} floor.min cases JS Math.floor(ms/60000) (double division) differs from the exact floor; the comparison uses exact floor for those cases')

    if a.perf:
        report['perf'] = perf(keys)
    (WORK / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=1))
    print(f'\nreport: {WORK / "report.json"}')
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
