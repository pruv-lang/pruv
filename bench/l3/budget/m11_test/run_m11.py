#!/usr/bin/env python3
"""L3-10 M11 driver: work cards (bud_card.pruv, RequestBudgets.setCard / hasCard / current / isHalted) against the oracle.

  python3 run_m11.py [--pairs py,rs] [--count 300] [--seed 20261117] [--perf] [--no-build]

1. copies root.pruv + bud_card.pruv and the modules it uses into work/m11/src/
2. compiles with the Python compiler (py/rust) and/or the Rust compiler (rs/rust), then rustc -O harness.rs per pair
3. cases: q_gen.gen(seed, ['Q6'], count); expected: oracle.run_oracle(cases) (the pinned TS)
4. replays each case through the harness (stand-in session store in root.pruv) and compares in canonical JV space:
   every request value (budget wrappers {bref, v}: the bref must be the named session's budget, then v is compared),
   per-request trace counts of findSession / now / save / changed, and finalSessions (id + budget).
   Every successful setCard budget must also match the blob dumped right after it, word for word (even when halted).
   Cases with a `halt` op are not-implemented (M11 has no halt): only the prefix before the first halt is replayed and
   reported as prefix-ok / prefix-fail; they never count as passes.
5. --perf: item C (diagnostic, Pruv vs hand only) = B without the stand-in store and out words; item A (validate+build, U = 100/300/1000) and item B (n = 100/500/1000 successive setCard) for Pruv, hand.rs
   and the pinned TS (perf_ts.mjs); items R (alternating cards), H (hasCard), I (setCard + current) for Pruv vs hand.
   Medians of >= 5 alternating runs.
6. --hdiff: hand.rs vs Pruv setCard on generated cards x scenarios (open / closed / missing / mismatched budget), tallied per error kind.
Report: work/m11/report.json. Exit 1 on any fail, 2 on a build or driver failure.
"""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import os
import random
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
import common                                # noqa: E402
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
def prepare_sources(work, typed):
    src = work / 'src'
    src.mkdir(parents=True, exist_ok=True)
    name = 'root_m11_typed.pruv' if typed else 'root.pruv'
    shutil.copy(HERE / name, src / name)
    for m in MODULES:
        shutil.copy(BUDGET / m, src / m)
    return src / name


def build_pair(key, root, work, typed):
    out = work / key
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
    main = main.replace('@TYPED@', str(HERE / 'harness_typed.rs'))
    (out / 'main.rs').write_text(main)
    cfg = ['--cfg', 'm11_typed'] if typed else []
    r = sh(['rustc', '-O', '--edition', '2021', '-A', 'warnings', *cfg, out / 'main.rs', '-o', out / 'harness'])
    if r.returncode != 0:
        sys.stderr.write(f'[{key}] rustc failed:\n{r.stderr[-3000:]}\n')
        sys.exit(2)
    return out / 'harness'


# ---- V words ---------------------------------------------------------------------------------------------------------------
def units(s):
    raw = s.encode('utf-16-le', errors='surrogatepass')
    return [struct.unpack_from('<H', raw, i)[0] for i in range(0, len(raw), 2)]


def pack(u):
    """Packed-u16 words (bud_keys.pruv): word i = u[4i]<<48 | u[4i+1]<<32 | u[4i+2]<<16 | u[4i+3], as a two's-complement
    i64 (u[4i] >= 0x8000 gives a negative word); lanes past len(u) are zero."""
    out = []
    for i in range(0, len(u), 4):
        x = 0
        for l in range(4):
            x = (x << 16) | (u[i + l] if i + l < len(u) else 0)
        out.append(x - (1 << 64) if x >= (1 << 63) else x)
    return out


def pkey(s):
    """Wire key `n, packed words(n)` of a string (packed once here; the harness and the root only move words)."""
    u = units(s)
    return [len(u)] + pack(u)


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


# ---- JB byte packets (card / lang / by; big-endian, see hand.rs header) --------------------------------------------------
def wtf8(s):
    """WTF-8 of a JS string held as a Python str (lone surrogates allowed): valid pairs join into 4-byte sequences."""
    return s.encode('utf-16-le', 'surrogatepass').decode('utf-16-le', 'surrogatepass').encode('utf-8', 'surrogatepass')


def _to_b(x, o):
    if x is UNDEF:
        o.append(1)
    elif x is None:
        o.append(2)
    elif x is False:
        o.append(3)
    elif x is True:
        o.append(4)
    elif isinstance(x, (int, float)):
        o.append(7)
        o += struct.pack('>d', float(x))
    elif isinstance(x, str):
        s = wtf8(x)
        o.append(6)
        o += struct.pack('>I', len(s)) + s
    elif isinstance(x, list):
        o.append(8)
        o += struct.pack('>I', len(x))
        for e in x:
            _to_b(e, o)
    elif isinstance(x, dict):
        o.append(9)
        o += struct.pack('>I', len(x))
        for k, e in x.items():
            s = wtf8(k)
            o += struct.pack('>I', len(s)) + s
            _to_b(e, o)
    else:
        raise TypeError(type(x))
    return o


def to_b(x):
    """JB packet of a JS value as a list of byte words (0..255)."""
    return list(_to_b(x, bytearray()))


def from_b(b, p=0):
    """Inverse of to_b (self-check only): (value, next position)."""
    t = b[p]
    if t == 1:
        return UNDEF, p + 1
    if t in (2, 3, 4):
        return (None, False, True)[t - 2], p + 1
    if t == 7:
        return struct.unpack('>d', bytes(b[p + 1:p + 9]))[0], p + 9
    if t == 6:
        n = struct.unpack('>I', bytes(b[p + 1:p + 5]))[0]
        return bytes(b[p + 5:p + 5 + n]).decode('utf-8', 'surrogatepass'), p + 5 + n
    if t in (8, 9):
        n, q = struct.unpack('>I', bytes(b[p + 1:p + 5]))[0], p + 5
        out = [] if t == 8 else {}
        for _ in range(n):
            if t == 8:
                e, q = from_b(b, q)
                out.append(e)
            else:
                kn = struct.unpack('>I', bytes(b[q:q + 4]))[0]
                k = bytes(b[q + 4:q + 4 + kn]).decode('utf-8', 'surrogatepass')
                out[k], q = from_b(b, q + 4 + kn)
        return out, q
    raise ValueError(f'bad tag {t} at {p}')


def selftest_b():
    cases = [UNDEF, None, True, False, 0, -0.0, 1.5, 2 ** 53, {'a': [1, 'x', None], 'b': {'c': ' é '}},
             '\U0001F600', '\ud83d', '\ude00', '\ud83d\ud83d', '\ude00\ud83d', {'k\ud800': 1}]
    for c in cases:
        b = to_b(c)
        assert all(0 <= w < 256 for w in b)
        got, end = from_b(b)
        assert end == len(b)
        assert canon(got) == canon(c), (c, got)
    assert to_b('😀') == [6, 0, 0, 0, 4, 0xF0, 0x9F, 0x98, 0x80]   # pair joined to one 4-byte sequence
    assert to_b('\ud83d') == [6, 0, 0, 0, 3, 0xED, 0xA0, 0xBD]               # lone surrogate stays 3 bytes
    assert to_b(True) == [4] and to_b(1) == [7, 0x3F, 0xF0, 0, 0, 0, 0, 0, 0]
    print('to_b selftest ok')


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


def sid_of(ref):
    """Packed key of ref.sessionId (the lookup key of the session table), packed once at ingress; [] when the ref is not an
    object with a string sessionId (then there is nothing to look up)."""
    s = ref.get('sessionId') if isinstance(ref, dict) else None
    return pkey(s) if isinstance(s, str) else []


# ---- one case ----------------------------------------------------------------------------------------------------------
def case_lines(case):
    """Harness input lines for one case, and per call the bookkeeping to read the answers back."""
    world = case['world']
    lines = ['init']
    for s in world.get('sessions', []):
        sess = jv_decode(s)
        b = sess.get('budget', UNDEF)
        lines.append(wline(pkey(sess['id']), [] if b is UNDEF else V(b)))
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
            card = to_b(args[1]) if len(args) > 1 else []
            lang = [] if len(args) < 3 else to_b(args[2])
            by = [] if len(args) < 4 else to_b(args[3])
            lines.append('set ' + wline([now], V(args[0]), sid_of(args[0]), card, lang, by))
            lines.append('dump')  # read-only; answers the store right after this set (identity oracle)
            calls.append({'req': ev['req'], 'op': ev['op'], 'ref': args[0], 'dump_after': True})
            continue
        elif ev['op'] in QUERY:
            lines.append('q ' + wline([QUERY[ev['op']]], V(args[0]), sid_of(args[0])))
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


def parse_dump(dump):
    """Dump words -> [(sid, blob)]. Strict framing: n >= 0, every key / blob length >= 0 and inside the dump, and the
    cursor ends exactly at len(dump), and every key unit in 0..65535. Raises ValueError on any violation."""
    if not dump:
        raise ValueError('empty dump')
    n, p, out = dump[0], 1, []
    if n < 0:
        raise ValueError(f'negative session count {n}')
    for _ in range(n):
        if p >= len(dump):
            raise ValueError('truncated before key length')
        kn = dump[p]
        if kn < 0 or p + 1 + kn > len(dump):
            raise ValueError(f'bad key length {kn} at word {p}')
        if any(not 0 <= x <= 65535 for x in dump[p + 1:p + 1 + kn]):
            raise ValueError(f'key unit out of range at word {p}')
        sid = from_units(dump[p + 1:p + 1 + kn])
        p += 1 + kn
        if p >= len(dump):
            raise ValueError('truncated before blob length')
        bn = dump[p]
        if bn < 0 or p + 1 + bn > len(dump):
            raise ValueError(f'bad blob length {bn} at word {p}')
        out.append((sid, dump[p + 1:p + 1 + bn]))
        p += 1 + bn
    if p != len(dump):
        raise ValueError(f'cursor ends at {p} of {len(dump)} words')
    return out


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
        dump_line = next(it) if c.get('dump_after') else None  # always consumed, whatever happens to the set answer
        blobs = None
        if dump_line is not None:
            # every dump after a set must be a well-formed ok dump, whether or not the set succeeded
            try:
                blobs = dict(parse_dump(parse_ok(dump_line)))
            except ValueError as e:
                problems.append(f"dump: malformed after {c['req']}: {str(e)[:200]}")
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
        if blobs is not None and isinstance(got, dict) and got.get('ok') is True:
            # setCard result ABI: 4 trace words and 13 words before the budget V; must equal the blob stored for sid.
            if sid not in blobs:
                problems.append(f"identity: no dumped blob for {sid!r} after {c['req']}")
            elif blobs[sid] != w[17:end]:
                problems.append(f"identity: returned budget differs from dumped blob for {sid!r} after {c['req']}")
        for b in brefs:
            if final_brefs.get(sid) != b:
                problems.append(f"{c['req']}: bref {b} is not the budget of session {sid!r}")
        if end != len(w):
            problems.append(f"{c['req']}: result V ends at word {end} of {len(w)}")
        if canon(got) != canon(want):
            problems.append(f"{c['req']}: value\n   want {canon(want)[:400]}\n   got  {canon(got)[:400]}")
        ec = counts.get(c['req'], {k: 0 for k in COUNTED})
        gc = {'findSession': finds, 'now': used_now, 'save': saved, 'changed': changed}
        if ec != gc:
            problems.append(f"{c['req']}: counts want {ec} got {gc}")
    final_line = next(it)
    try:  # the final dump must be well formed even when the replay halted (only the comparison is skipped)
        entries = parse_dump(parse_ok(final_line))
    except ValueError as e:
        entries = None
        problems.append(f"dump: malformed final dump: {str(e)[:200]}")
    if not halted:
        if entries is not None:
            got_s = []
            for sid, blob in entries:
                s = {'id': sid}
                if blob:
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
        [(item, n, max(5, 20000 // n)) for item in ('C', 'R', 'H', 'I') for n in (100, 1000)]
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
            if item in ('A', 'B'):
                r = sh(node + [item, size, reps], cwd=BUDGET, env=TS_ENV())
                tv.append(ns_of(r.stdout.strip() or 'TS failed: ' + r.stderr[-500:]))
        p, h = statistics.median(pv), statistics.median(hv)
        t = statistics.median(tv) if tv else float('nan')
        rows.append({'item': item, 'size': size, 'pruv_ns': p, 'hand_ns': h, 'ts_ns': t, 'pruv_hand': round(p / h, 2)})
        print(f'  {item} | {size:5} | {p:9.1f} | {h:9.1f} | {t:9.1f} | {p / h:.2f}')
    return rows


# ---- --hdiff: hand.rs vs Pruv on generated cards ----------------------------------------------------------------------------
def hdiff_items():
    """[(card, lang, by, scen, label)]: the generated card corpus (scen 0), its closed-budget copies, crafted error cards."""
    items = []

    def add(card, label, lang=None, by=None):
        items.append((list(card), list(lang or []), list(by or []), 0, label))

    def card(d='d', done=('x',), **kw):
        return to_b({'deliverable': d, 'done': list(done), **kw})

    def from_cases(cases, label):
        for c in cases:
            for ev in c['script']:
                if ev.get('ev') == 'call' and ev.get('op') == 'setCard':
                    a = [jv_decode(x) for x in ev['args']]
                    if len(a) < 2:
                        continue
                    add(to_b(a[1]), label, to_b(a[2]) if len(a) > 2 else [], to_b(a[3]) if len(a) > 3 else [])

    for seed in (20261117, 1, 2, 3):
        from_cases(q_gen.gen(seed, ['Q6'], count=300), 'Q6')
    from_cases([common.strip_case(c) for c in common.load_group('K')], 'K')

    ws = [' ', '\t', '\n', '\x0b', '\x0c', '\r', '\xa0', ' ', ' ', ' ', ' ', ' ', ' ', ' ', '　', '﻿', '​', '\u0085']
    for w in ws:
        for d in (w + 'a' + w, w * 3, 'a' + w * 2 + 'b', w + '\ud83d' + w):
            add(card(d), 'trim')
            add(card('ok', done=[d]), 'trim')
    for n in (299, 300, 301):
        for unit in ('a', 'é', '中', '😀', '\ud83d', '\ude00'):
            s = unit * (n // (2 if unit == '😀' else 1))
            add(card(s), 'len')
            add(card(' ' + s + ' '), 'len')
            add(card(targets={'confirmed': 1, 'candidate': 0, 'unknown': 0, 'note': s}), 'note')
            add(card(targets={'confirmed': 1, 'candidate': 0, 'unknown': 0, 'note': ' ' + s}), 'note')
    add(card('😀' * 150 + 'a'), 'len')
    for head in ('a' * 299, 'a' * 300, 'a' * 301, 'a' * 298 + '\U0001f600', 'a' * 299 + '\U0001f600', '\U0001f600' * 150, '中' * 300, 'a' * 299 + '　'):
        for tail in ('', ' ' * 5000, '　' * 400 + '﻿', ' ' * 300 + 'b', '　' * 2 + '\U0001f600', ' ' * 3 + '\ud83d'):
            add(card(head + tail), 'cut')
            add(card('ok', done=['\t' + head + tail]), 'cut')

    def numcard(bits_list):
        o = bytearray(card())
        o[1:5] = struct.pack('>I', 3)
        o += struct.pack('>I', 7) + b'targets' + bytes([9]) + struct.pack('>I', 3)
        for k, b in zip(('confirmed', 'candidate', 'unknown'), bits_list):
            o += struct.pack('>I', len(k)) + k.encode() + bytes([7]) + struct.pack('>Q', b)
        return o

    def f(x):
        return struct.unpack('>Q', struct.pack('>d', x))[0]

    inf = float('inf')
    nums = [0, f(-0.0), f(1.0), f(-1.0), f(0.5), f(2.0 ** 53), f(2.0 ** 53 - 1), f(2.0 ** 53 + 2), f(1e300), f(-1e300), f(inf), f(-inf),
            0x7ff8000000000000, 0x7ff0000000000001, 0xfff8000000000000, 1, 0x8000000000000001, f(2.0 ** 63), f(-2.0 ** 63),
            f(4503599627370495.5), f(2.0 ** 52 + 0.5)]
    for a in nums:
        for b in (0, a):
            add(numcard([a, b, 0]), 'num')
            add(numcard([0, 0, a]), 'num')

    for by in ({'sessionId': 's', 'turnId': 't'}, {'sessionId': 1.5, 'turnId': -0.0}, {'sessionId': 2.0 ** 60, 'turnId': [1, {'a': None}]},
               {}, '', 'x', 0, -0.0, float('nan'), 1, True, False, None, UNDEF, [], {'turnId': 't', 'sessionId': 's', 'sessionId2': 1}):
        add(card(), 'by', by=to_b(by))
        add(card(state='done'), 'by', lang=to_b('en'), by=to_b(by))
    for lang in ('en', 'ko', 'EN', '', 1, None):
        add(card(''), 'lang', lang=to_b(lang))
    for v in ([], [{'label': 'a', 'status': 'done'}] * 5, [{'label': 'a', 'status': 'done'}] * 6, [{'label': ' ', 'status': 'done'}],
              [{'label': 'a', 'status': 'Done'}], [{'status': 'planned', 'label': 'x', 'extra': 1}], [1], 'x', {}, None):
        add(card(verify=v), 'verify')
    for st in ('working', 'done', 'waiting-user', 'blocked', 'closed', '', 1, None, UNDEF):
        add(card(state=st), 'state')
    for cf in ([], ['a'] * 5, ['a'] * 6, [' '], [1], 'a', None, UNDEF):
        add(card(conflicts=cf), 'conflicts')
    dup = bytearray(to_b({'deliverable': ' ', 'done': ['x']}))
    dup[1:5] = struct.pack('>I', 3)
    dup += struct.pack('>I', 11) + b'deliverable' + bytes(to_b('ok'))
    add(dup, 'dup')
    add(to_b({'deliverable': UNDEF, 'done': ['x']}), 'undef')
    add(card(targets=UNDEF, verify=UNDEF), 'undef')
    add(to_b([1]), 'shape')
    add(to_b('x'), 'shape')
    add(to_b(None), 'shape')
    add([], 'empty')
    for d in (30, 31, 32, 33):
        x = 1
        for _ in range(d - 2):
            x = [x]
        add(card(by_extra=x), 'depth')
        add(card(), 'depth', by=to_b({'sessionId': x, 'turnId': 1}))

    def inj(seq, where):
        c = bytearray(card('ab', done=['cd'], targets={'confirmed': 1, 'candidate': 0, 'unknown': 0, 'note': 'ef'}))
        tag = {'d': b'ab', 'l': b'cd', 'n': b'ef'}[where]
        p = c.index(bytes([6, 0, 0, 0, 2]) + tag) + 5
        c[p + 1:p + 1] = seq
        c[p - 4:p] = struct.pack('>I', struct.unpack('>I', c[p - 4:p])[0] + len(seq))
        return c

    enc = [b'\xc0\x80', b'\xc1\xbf', b'\xe0\x80\x80', b'\xe0\x9f\xbf', b'\xe0\xa0\x80', b'\xed\xa0\x80', b'\xed\xbf\xbf',
           b'\xed\xa0\x80\xed\xb0\x80', b'\xed\xaf\xbf\xed\xbf\xbf', b'\xed\xb0\x80\xed\xa0\x80', b'\xed\xa0\x80a\xed\xb0\x80',
           b'\xf0\x80\x80\x80', b'\xf0\x8f\xbf\xbf', b'\xf0\x90\x80\x80', b'\xf4\x8f\xbf\xbf', b'\xf4\x90\x80\x80', b'\xf5\x80\x80\x80',
           b'\x80', b'\xbf', b'\xc2', b'\xe2\x80', b'\xf0\x9f\x98', b'\xff', b'\xc2\xa0', b'\xef\xbb\xbf']
    for seq in enc:
        for wh in 'dln':
            add(inj(seq, wh), 'enc')
        add(card(), 'enc', by=bytes([6]) + struct.pack('>I', len(seq)) + seq)
        add(card(), 'enc', lang=bytes([6]) + struct.pack('>I', len(seq)) + seq)

    rng = random.Random(25)
    valid = [it for it in items if it[4] in ('Q6', 'K')][:400]
    bad_utf = [b'\xc0\x80', b'\xe0\x80\x80', b'\xed\xa0\x80\xed\xb0\x80', b'\xf4\x90\x80\x80', b'\xf5', b'\x80', b'\xe2\x80', b'\xf0\x9f\x98']
    for i in range(3000):
        c = bytearray(rng.choice(valid)[0])
        if not c:
            continue
        r = rng.random()
        if r < 0.3:
            c[rng.randrange(len(c))] = rng.randrange(256)
        elif r < 0.5:
            del c[rng.randrange(len(c)):]
        elif r < 0.6:
            c += bytes([rng.randrange(256)])
        else:
            c = bytearray(card('a' + 'b' * rng.randrange(3)))
            bad = rng.choice(bad_utf)
            s0 = 5 + 4 + 11
            assert c[s0] == 6
            p = s0 + 5 + rng.randrange(struct.unpack('>I', c[s0 + 1:s0 + 5])[0] + 1)
            c[p:p] = bad
            c[s0 + 1:s0 + 5] = struct.pack('>I', struct.unpack('>I', c[s0 + 1:s0 + 5])[0] + len(bad))
        add(c, 'mut')

    # a) every ported item again against a closed budget
    items += [(c, l, b, 1, 'closed:' + lab) for (c, l, b, _, lab) in list(items)]

    # b) crafted error cards per kind, crossed with lang and scenario
    def tg(**kw):
        return {'confirmed': 1, 'candidate': 0, 'unknown': 0, 'note': '', **kw}

    crafted = [to_b({'deliverable': d, 'done': ['x']}) for d in ('', '   ', 'a' * 301, 5)] + [[]]
    crafted += [card(done=d) for d in ([], ['x'] * 6, ['a' * 301], [5])]
    crafted += [card(targets=tg(confirmed=-1)), card(targets=tg(confirmed=0.5)), card(targets=tg(note='n' * 301)),
                card(targets=tg(candidate='x'))]
    crafted += [card(verify=[{'label': 'a', 'status': 'done'}] * 6), card(verify=[{'label': '', 'status': 'done'}]),
                card(verify=[{'label': 'a', 'status': 'x'}])]
    crafted += [card(conflicts=['a'] * 6), card(conflicts=['a' * 301])]
    crafted += [card(state=s) for s in ('closed', 'x', 1)]
    crafted += [card()]
    for c in crafted:
        for lang in ('en', 'ko', 'EN', '', 1, None):
            for scen in range(5):
                items.append((c, to_b(lang), [], scen, 'kind'))
        for scen in range(5):
            items.append((c, [], [], scen, 'kind'))
    return items


def hdiff_pair(k, harness):
    items = hdiff_items()
    data = ''.join('hdiff ' + wline(c, l, b, [s]) + '\n' for (c, l, b, s, _) in items)
    r = sh([harness], data=data)
    if r.returncode != 0:
        sys.stderr.write(r.stderr[-2000:])
        sys.exit(2)
    outs = r.stdout.splitlines()
    tally, errs, differ = {}, {}, []
    for (c, l, b, s, lab), line in zip(items, outs):
        p = line.split()
        if p[:2] == ['same', 'err'] and len(p) == 5 and p[4] != str(int(int(p[2]) >= 3)):
            differ.append(f'{lab} scen={s}: used_now {p[4]} for kind {p[2]}')
            continue
        if p[:2] == ['same', 'err'] and len(p) == 5:
            outcome = f'err {p[2]} en={p[3]} used={p[4]}'
            cell = errs.setdefault((int(p[2]), int(p[3])), {})
            cell[p[4]] = cell.get(p[4], 0) + 1
        elif line == 'same ok':
            outcome = 'ok'
        elif line == 'same refused':
            outcome = 'refused'
        else:
            differ.append(f'{lab} scen={s}: {line[:300]}')
            continue
        key = f'{lab.split(":")[-1]}|{s}|{outcome}'
        tally[key] = tally.get(key, 0) + 1
    bad = bool(differ) or len(outs) != len(items)
    print(f'[{PAIRS[k]}] hdiff items={len(items)} lines={len(outs)} differ={len(differ)}')
    for d in differ[:15]:
        print('  differ', d)
    print('  kind en | count | used_now values')
    for kind in range(7):
        for en in (0, 1):
            cell = errs.get((kind, en), {})
            if not cell:
                bad = True
            print(f'  {kind}    {en}  | {sum(cell.values()):5} | {dict(sorted(cell.items()))}')
    (WORK / f'hdiff_{k}.json').write_text(json.dumps(
        {'items': len(items), 'lines': len(outs), 'differ': len(differ), 'tally': tally,
         'errors': {f'{kd}|{en}': v for (kd, en), v in sorted(errs.items())}}, indent=1, ensure_ascii=False))
    return bad


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--pairs', default='py,rs')
    ap.add_argument('--count', type=int, default=300)
    ap.add_argument('--seed', type=int, default=q_gen.SEED if hasattr(q_gen, 'SEED') else 20261117)
    ap.add_argument('--perf', action='store_true')
    ap.add_argument('--typed', action='store_true', help='use the typed resident session map')
    ap.add_argument('--runs', type=int, default=5)
    ap.add_argument('--no-build', action='store_true')
    ap.add_argument('--hdiff', action='store_true', help='hand vs Pruv setCard on generated cards x scenarios (no oracle replay)')
    ap.add_argument('--selftest-b', action='store_true', help='check the to_b codec and exit')
    a = ap.parse_args()
    if a.selftest_b:
        selftest_b()
        return
    work = WORK.with_name('m11_typed') if a.typed else WORK
    work.mkdir(parents=True, exist_ok=True)
    pairs = [p.strip() for p in a.pairs.split(',') if p.strip()]
    root = prepare_sources(work, a.typed)
    harness = {}
    for k in pairs:
        h = work / k / 'harness'
        harness[k] = h if (a.no_build and h.exists()) else build_pair(k, root, work, a.typed)
    if a.hdiff:
        bad = False
        for k in pairs:
            bad = hdiff_pair(k, harness[k]) or bad
        sys.exit(1 if bad else 0)
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
    (work / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False))
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
