"""L3-6 Grok import: builds the four verified pairs (py/rust, py/llvm, rs/rust, rs/llvm), runs every fixture of fixtures/*.json
on the pinned TypeScript (Node, reference.mjs) and on each pair, and compares them with oracle.compare (results, ACP transcript,
primitive trace, final trees). A difference passes only when oracle.py names the policy whose trigger is in the fixture.
Only this piece is built; no repository-wide suites.

    PATH="$HOME/.cargo/bin:$PATH" GROK_TS_TRANSPILER=/path/to/typescript python3 bench/l3/grok/test_impl.py
        [--verify-only] [--reuse-verified] [--only SUBSTRING]... [--category NAME]... [--pairs py/rust,rs/llvm] [--no-ts]
        [--jobs N] [--slow] [--keep] [--list]

This module is also the fixture library of q.py and q_mutants.py (load_fixtures, expand_case, run_reference, judge, ...).

Fixture files: fixtures/_defaults.json (per-mode defaults) and fixtures/*.json (a JSON list of cases). A case is the common.py
fixture plus these conveniences, all expanded here (nothing of this reaches the host, the reference or oracle.py):
  "gen": [ {"session": {...}} | {"sessions": {...}} ]   bulk session trees (see expand_session / expand_sessions)
  "kills": ["M01"]    the mutants this fixture is meant to kill (checked by q_mutants.py)
  "disabled": "why"   skipped (kept as documentation of a case that needs a host/reference change)
  "slow": true        skipped unless --slow
  "expected": {status, class, code, stage, errnoClass, nrows, skipped, present, omitted, title, model}   checked against the
                      pinned TypeScript result (the oracle), never against the Pruv side alone.
"""
import sys
sys.dont_write_bytecode = True
import argparse
import copy
import importlib.util
import json
import os
import random
import re
import time
import urllib.parse
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from common import HERE, WORK, bytes_of, clean, reference_command, run_side
from oracle import compare, norm_result, p14_case, p1_case, p4_case, triggers, unexplained

FIXTURES = HERE / 'fixtures'
PAIRS = [(c, b) for c in ('py', 'rs') for b in ('rust', 'llvm')]


# ---- fixtures: expansion of the conveniences --------------------------------------------------------------------------
def enc_folder(folder):
    """encodeURIComponent for valid Unicode: UTF-8 percent bytes, uppercase hex, A-Za-z0-9 - _ . ! ~ * ' ( ) kept."""
    return urllib.parse.quote(folder, safe="-_.!~*'()", encoding='utf-8', errors='strict')


def dump(v):
    """JSON text of v. {"$raw": TEXT} is inserted verbatim (1e400, -0, \\ud800 escapes, invalid JSON); never round-trips numbers."""
    if isinstance(v, dict):
        if '$raw' in v:
            return v['$raw']
        return '{' + ','.join(json.dumps(k, ensure_ascii=False) + ':' + dump(x) for k, x in v.items()) + '}'
    if isinstance(v, list):
        return '[' + ','.join(dump(x) for x in v) + ']'
    return json.dumps(v, ensure_ascii=False)


def utf8(s):
    return {'utf8': s}


def line_pieces(spec):
    """{"type","prefix","size","cr","nolf","extra","pre","lead","typekey","indent"}: one history line of exactly `size` bytes (LF and CR
    included): <indent>{<lead>"<typekey>":"<type>",<pre>"content":"<prefix><x padding>"<extra>}[CR][LF].
    `lead`/`pre`/`extra` are raw JSON text (`"a":1,` / `"synthetic_reason":0,` / `,"synthetic_reason":"x"`)."""
    head = '%s{%s"%s":"%s",%s"content":"%s' % (spec.get('indent', ''), spec.get('lead', ''), spec.get('typekey', 'type'), spec.get('type', 'user'),
                                               spec.get('pre', ''), spec.get('prefix', ''))
    tail = '"%s}%s%s' % (spec.get('extra', ''), '\r' if spec.get('cr') else '', '' if spec.get('nolf') else '\n')
    pad = spec['size'] - len(head.encode()) - len(tail.encode())
    if pad < 0:
        raise ValueError('line size %d too small for %r' % (spec['size'], spec))
    return [utf8(head), {'repeat': utf8(spec.get('ch', 'x')), 'count': pad}, utf8(tail)]


def history_pieces(items):
    """History elements -> recipe pieces. Element: a JSON object (one line), {"$raw": TEXT, "$nolf": bool}, {"$hex": HEX} (raw bytes),
    {"$line": {...}} (see line_pieces) or {"$repeat": ELEMENT, "count": N}."""
    out = []
    for it in items:
        if '$repeat' in it:
            inner = it['$repeat'] if isinstance(it['$repeat'], list) else [it['$repeat']]
            out.append({'repeat': {'concat': history_pieces(inner)}, 'count': it['count']})
        elif '$raw' in it:
            out.append(utf8(it['$raw'] + ('' if it.get('$nolf') else '\n')))
        elif '$hex' in it:
            out.append({'hex': it['$hex']})
        elif '$line' in it:
            out += line_pieces(it['$line'])
        else:
            out.append(utf8(dump(it) + '\n'))
    return out


def merge_over(base, over):
    """dict overrides; {"$del":1} deletes a key; a dict value of `info` merges into the default info."""
    out = dict(base)
    for k, v in over.items():
        if v == {'$del': 1}:
            out.pop(k, None)
        elif k == 'info' and isinstance(v, dict) and '$raw' not in v and isinstance(out.get('info'), dict):
            out['info'] = merge_over(out['info'], v)
        else:
            out[k] = v
    return out


def default_summary(thread, cwd):
    return {'info': {'id': thread, 'cwd': cwd}, 'generated_title': 'Title ' + thread, 'updated_at': 1700000000000, 'num_chat_messages': 3}


def summary_recipe(s, thread, folder):
    cwd = s.get('cwd', folder)
    d = merge_over(default_summary(thread, cwd), s.get('summary', {}))
    if 'summary_size' in s:                       # pad to an exact file size (65536 / 65537 boundaries)
        text = dump(dict(d, pad=''))
        need = s['summary_size'] - len(text.encode())
        if need < 0:
            raise ValueError('summary_size %d too small' % s['summary_size'])
        d = dict(d, pad='x' * need)
    return utf8(dump(d))


def expand_session(s, case, tree, sched):
    """{"session": {home, folder, enc, cwd, thread, summary:{overrides}, summary_raw, summary_size, no_summary, history:[...],
    history_raw, files:{name: recipe | tree entry}, link, dir_mode, readdir, also:[names]}} -> tree entries (+ readdir schedule)."""
    home = s.get('home', '$SRC')
    folder = s.get('folder', case.get('folder', '/work'))
    thread = s['thread']
    base = '%s/sessions/%s' % (home, s.get('enc', enc_folder(folder)))
    path = '%s/%s' % (base, thread)
    if 'link' in s:
        tree.append({'path': path, 'kind': 'symlink', 'target': s['link']})
        return
    entry = {'path': path, 'kind': 'dir'}
    if 'dir_mode' in s:
        entry['mode'] = s['dir_mode']
    tree.append(entry)
    names = []
    if not s.get('no_summary'):
        names.append('summary.json')
        tree.append({'path': path + '/summary.json', 'kind': 'file', 'bytes': s['summary_raw'] if 'summary_raw' in s else summary_recipe(s, thread, folder)})
    if 'history' in s or 'history_raw' in s:
        names.append('chat_history.jsonl')
        rec = s['history_raw'] if 'history_raw' in s else {'concat': history_pieces(s['history'])}
        tree.append({'path': path + '/chat_history.jsonl', 'kind': 'file', 'bytes': rec})
    for name, v in s.get('files', {}).items():
        names.append(name)
        if isinstance(v, dict) and 'kind' in v:
            tree.append(dict(v, path=path + '/' + name))
        else:
            tree.append({'path': path + '/' + name, 'kind': 'file', 'bytes': utf8(v) if isinstance(v, str) else v})
    if s.get('readdir'):
        sched.setdefault('readdir', []).append({'path': path, 'names': names + s.get('also', [])})


def make_dates(spec, n):
    """Per-id raw date tokens (None = no updated_at key). spec: list | "asc" | "desc" | {seed, missing_pct, base, distinct}."""
    if isinstance(spec, list):
        return list(spec) + [None] * (n - len(spec))
    if spec == 'asc':
        return [str(1700000000000 + i * 1000) for i in range(n)]
    if spec == 'desc':
        return [str(1700000000000 + (n - i) * 1000) for i in range(n)]
    rng = random.Random(spec.get('seed', 1))
    out = []
    for _ in range(n):
        if rng.random() * 100 < spec.get('missing_pct', 0):
            out.append(None)
        else:
            out.append(str(spec.get('base', 1700000000000) + rng.randrange(spec.get('distinct', 1000)) * 60000))
    return out


def expand_sessions(s, case, tree, sched):
    """{"sessions": {home, folder, ids | prefix+n, dates, summary:{common overrides}, summary_by_id:{id:{...}}, also, order}} ->
    one session dir with a summary per id plus the readdir schedule of the encoded folder directory."""
    home = s.get('home', '$SRC')
    folder = s.get('folder', case.get('folder', '/work'))
    ids = s['ids'] if 'ids' in s else ['%s%02d' % (s.get('prefix', 'S'), i) for i in range(s['n'])]
    dates = make_dates(s.get('dates', 'asc'), len(ids))
    base = '%s/sessions/%s' % (home, s.get('enc', enc_folder(folder)))
    for i, (sid, tok) in enumerate(zip(ids, dates)):
        tree.append({'path': '%s/%s' % (base, sid), 'kind': 'dir'})
        d = default_summary(sid, s.get('cwd', folder))
        d = merge_over(d, {'updated_at': {'$del': 1} if tok is None else {'$raw': tok}, 'num_chat_messages': i})
        d = merge_over(d, s.get('summary', {}))                     # sequential: a later {"$del":1} must reach the default keys
        d = merge_over(d, s.get('summary_by_id', {}).get(sid, {}))
        tree.append({'path': '%s/%s/summary.json' % (base, sid), 'kind': 'file', 'bytes': utf8(dump(d))})
    names = s.get('order') or (ids + s.get('also', []))
    if s.get('readdir', True):
        sched.setdefault('readdir', []).append({'path': base, 'names': names})


def merge_case(case, defaults):
    out = copy.deepcopy(defaults.get(case['mode'], {}))
    for k, v in case.items():
        if k in ('tree', 'gen'):
            out[k] = out.get(k, []) + copy.deepcopy(v)
        elif k in ('schedule', 'source', 'acp', 'acp_options', 'expected', 'concurrency') and isinstance(v, dict):
            out.setdefault(k, {}).update(copy.deepcopy(v))
        else:
            out[k] = copy.deepcopy(v)
    return out


def macro(v):
    """{"$rep": "x", "n": 129} -> "x"*129; {"$cat": [A, B, ...]} -> the strings joined (any string field of a fixture)."""
    if isinstance(v, dict):
        if set(v) == {'$rep', 'n'}:
            return v['$rep'] * v['n']
        if set(v) == {'$cat'}:
            return ''.join(macro(x) for x in v['$cat'])
        return {k: macro(x) for k, x in v.items()}
    if isinstance(v, list):
        return [macro(x) for x in v]
    return v


def expand_case(case, defaults=None):
    case = macro(merge_case(case, defaults or {}))
    tree = list(case.get('tree', []))
    sched = case.setdefault('schedule', {})
    sched.setdefault('readdir', [])
    for g in case.pop('gen', []):
        if 'session' in g:
            expand_session(g['session'], case, tree, sched)
        elif 'sessions' in g:
            expand_sessions(g['sessions'], case, tree, sched)
        else:
            raise ValueError('bad gen directive %r' % list(g))
    case['tree'] = tree
    if not sched['readdir']:
        del sched['readdir']
    for acts in case.get('acp', {}).values():       # "rawhex_of": TEXT -> "rawhex" (oracle.triggers only looks at rawhex for P1)
        for a in acts if isinstance(acts, list) else [acts]:
            if isinstance(a, dict) and 'rawhex_of' in a:
                a['rawhex'] = a.pop('rawhex_of').encode('utf-8').hex()
    return case


def prepare(case):
    """The copy that is run: bytes/blob recipes of schedule actions become hex (host.rs reads only utf8/hex)."""
    c = copy.deepcopy(case)
    blobs = c.get('blobs', {})
    for r in c.get('schedule', {}).get('rules', []):
        for a in r.get('actions', []):
            if 'bytes' in a and not (isinstance(a['bytes'], dict) and set(a['bytes']) <= {'utf8', 'hex'}):
                a['bytes'] = {'hex': bytes_of(a['bytes'], blobs).hex()}
    if blobs:
        c['blobs'] = {k: {'hex': bytes_of(v, blobs).hex()} for k, v in blobs.items()}
    return c


def load_fixtures(only=(), categories=(), slow=False, include_disabled=False):
    """All fixtures of fixtures/*.json, expanded. Each case gets `category` (file stem). Returns (cases, skipped)."""
    defaults = json.loads((FIXTURES / '_defaults.json').read_text(encoding='utf-8'))
    cases, skipped, seen = [], [], set()
    for f in sorted(FIXTURES.glob('*.json')):
        if f.name.startswith('_'):
            continue
        raw = json.loads(f.read_text(encoding='utf-8'))
        for c in (raw['cases'] if isinstance(raw, dict) else raw):
            c = dict(c, category=f.stem)
            if c['name'] in seen:
                raise ValueError('duplicate fixture name ' + c['name'])
            seen.add(c['name'])
            if categories and f.stem not in categories:
                continue
            if only and not any(o in c['name'] for o in only):
                continue
            if c.get('disabled') and not include_disabled:
                skipped.append((c['name'], 'disabled: ' + str(c['disabled'])))
                continue
            if c.get('slow') and not slow:
                skipped.append((c['name'], 'slow (use --slow)'))
                continue
            cases.append(expand_case(c, defaults))
    return cases, skipped


# ---- running and judging one case ------------------------------------------------------------------------------------
def run_reference(case, root=None):
    """(ts run, baselines or None) of a case on the pinned TypeScript. Baselines are TS reruns on the case with only a
    policy's trigger input edited: {'P4': unsupported raw date tokens (oracle.p4_case), 'P1': JSON the shared parser
    rejects (oracle.p1_case), 'P1+P4': both edits}; each is made only when the case has that trigger."""
    run = prepare(case)
    ts = run_side(run, 'ts', reference_command(), root=root)
    base = {}
    trig = triggers(case)
    for name, make in (('P4', p4_case), ('P1', p1_case)):
        b = make(run) if name in trig else None
        if b is not None:
            base[name] = run_side(b, 'ts', reference_command())
    if 'P4' in base and 'P1' in base:
        # both edits on one case: a rejected line and an unsupported date in the same input
        b = p14_case(run)
        if b is not None:
            base['P1+P4'] = run_side(b, 'ts', reference_command())
    return ts, base or None


def run_pair(case, side, exe, root=None):
    return run_side(prepare(case), side, [exe], root=root)


def check_expected(case, ts):
    """The `expected` block against the pinned TypeScript result (a fixture that does not do what it says is a fixture bug)."""
    exp = case.get('expected')
    if not exp or ts.get('result') is None:
        return []
    r = norm_result(ts['result'], ts['roots'])
    err = r.get('error') or {}
    got = {'status': r['status'], 'class': err.get('class'), 'code': err.get('code'), 'stage': err.get('stage'), 'errnoClass': err.get('errnoClass'),
           'nrows': len(r.get('rows') or []), 'skipped': r.get('copySkipped'), 'present': r.get('present'), 'omitted': r.get('omittedBefore'),
           'title': r.get('title'), 'model': r.get('model')}
    return ['expected %s=%r but the reference gave %r' % (k, v, got.get(k)) for k, v in exp.items() if got.get(k) != v]


def judge(case, ts, base, impl):
    """-> {ok, policies, notes, problems}: oracle.compare of the case; unexplained differences are problems."""
    diffs = compare(case, ts, impl, base)
    bad = unexplained(diffs)
    return {
        'ok': not bad, 'problems': ['%s: %s' % (d['area'], d['detail'][:300]) for d in bad],
        'policies': sorted({p for d in diffs for p in (d['policy'] or '').split('+') if p}),
        'notes': ['%s: %s' % (d['area'], d['detail'][:120]) for d in diffs if d['severity'] != 'diff' or d['policy']],
    }


def impl_clean(impl):
    """A runner that exited normally and was not killed by the watchdog (crashes and watchdog kills are never a kill)."""
    return impl.get('returncode') in (0, None) and not impl.get('watchdog') and impl.get('result') is not None


# ---- build ------------------------------------------------------------------------------------------------------------
def configure():
    spec = importlib.util.spec_from_file_location('super_common', HERE.parent / 'supervisor/common.py')
    c = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(c)
    g, run = c.configure(WORK)
    return c, g, run


def newest(paths):
    paths = list(paths)
    if not paths:
        raise FileNotFoundError('no matching rlib; run test_impl.py first')
    return max(paths, key=lambda p: p.stat().st_mtime)


def host_artifacts():
    deps = WORK / 'host/debug/deps'
    return deps, newest(deps.glob('libgrok_host-*.rlib')), newest(deps.glob('libserde_json-*.rlib')), WORK / 'libpruv_str.rlib'


def build_host(g, run):
    """Host crate (cargo, offline) and the string adapter; returns (deps dir, host rlib, serde_json rlib, adapter rlib)."""
    run([str(Path.home() / '.cargo/bin/cargo'), 'build', '--offline', '--manifest-path', HERE / 'host/Cargo.toml', '--target-dir', WORK / 'host'])
    adapter = WORK / 'libpruv_str.rlib'
    run([g.RUSTC, '--crate-name', 'pruv_str', '--crate-type', 'rlib', HERE / 'str_adapter.rs', '-o', adapter])
    return host_artifacts()


def compile_flags(adapter, manifest=None):
    return ['--rust-extern', f'pruv_str={adapter}', '--manifest', str(manifest or HERE / 'pruv.json')]


def link_runner(g, run, built, out, host, backend):
    deps, lib, serde, adapter = host
    harness = out / 'main.rs'
    harness.write_text((HERE / 'harness.rs').read_text().replace('@SOL@', built['rs']))
    extra = []
    if built['ll']:
        extra, err = g.build_llvm_objects(built['ll'], 'grok', str(out))
        assert not err, err
    exe = out / 'runner'
    run([g.RUSTC, '-O', '--edition', '2021', harness, '-o', exe, '--extern', f'grok_host={lib}', '--extern', f'pruv_str={adapter}',
         '--extern', f'serde_json={serde}', '-L', f'dependency={deps}', *extra])
    return exe


def build_pair(g, run, compiler, backend, host, reuse=False, verify_only=False):
    """Verify + compile root.pruv (verified mode, G4b manifest, nothing passes via --no-verify) for one pair and link the runner."""
    out = WORK / f'{compiler}_{backend}'
    out.mkdir(exist_ok=True)
    if reuse:
        built = {'rc': 0, 'rs': str(out / 'root_gen.rs'), 'll': str(out / 'root_gen.ll') if backend == 'llvm' else None}
        assert Path(built['rs']).is_file(), 'run once without --reuse-verified first'
    else:
        built = g.compile_pruv(compiler, str(HERE / 'root.pruv'), str(out), backend, compile_flags(host[3]))
    if built['rc']:
        raise RuntimeError('compile/verify failed: ' + str(built.get('errors') or built.get('stderr') or built))
    print(f'{compiler}/{backend}: ' + ('reusing verified Pruv emission' if reuse else 'verified'), flush=True)
    return None if verify_only else link_runner(g, run, built, out, host, backend)


# ---- main -------------------------------------------------------------------------------------------------------------
def parse_pairs(text):
    if not text:
        return PAIRS
    out = []
    for p in text.split(','):
        c, b = p.strip().split('/')
        if (c, b) not in PAIRS:
            raise SystemExit('unknown pair ' + p)
        out.append((c, b))
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--verify-only', action='store_true', help='build and verify the pairs, run nothing')
    ap.add_argument('--reuse-verified', action='store_true', help='reuse work/<pair>/root_gen.* of an earlier verified run')
    ap.add_argument('--only', action='append', default=[], help='fixtures whose name contains SUBSTRING (repeatable)')
    ap.add_argument('--category', action='append', default=[], help='fixture file stem (paths, identity, ...; repeatable)')
    ap.add_argument('--pairs', help='comma list of py/rust,py/llvm,rs/rust,rs/llvm (default all four)')
    ap.add_argument('--no-ts', action='store_true', help='no TypeScript run: only the `expected` blocks are checked, on the pairs')
    ap.add_argument('--ts-transpiler', help='typescript package for the reference (else $GROK_TS_TRANSPILER)')
    ap.add_argument('--jobs', type=int, default=4)
    ap.add_argument('--slow', action='store_true', help='include slow fixtures (the real 20 s deadline)')
    ap.add_argument('--keep', action='store_true', help='keep the work trees of passing cases')
    ap.add_argument('--list', action='store_true', help='list the selected fixtures and exit')
    args = ap.parse_args(argv)
    cases, skipped = load_fixtures(args.only, args.category, args.slow)
    if args.list:
        for c in cases:
            print('%-16s %-44s kills=%s' % (c['category'], c['name'], ','.join(c.get('kills', []))))
        for name, why in skipped:
            print('skipped %-44s %s' % (name, why))
        return 0
    if args.ts_transpiler:
        os.environ['GROK_TS_TRANSPILER'] = args.ts_transpiler
    if not args.no_ts and not args.verify_only and not os.environ.get('GROK_TS_TRANSPILER'):
        print('GROK_TS_TRANSPILER (or --ts-transpiler) must name the typescript package: the reference transpiles plan.ts with it', file=sys.stderr)
        return 2
    c, g, run = configure()
    host = build_host(g, run)
    pairs = parse_pairs(args.pairs)
    exes = {}
    for compiler, backend in pairs:
        exes[(compiler, backend)] = build_pair(g, run, compiler, backend, host, args.reuse_verified, args.verify_only)
    if args.verify_only:
        return 0
    failures, report = [], {'fixtures': len(cases), 'skipped': skipped, 'pairs': {}, 'ts': {}}
    refs = {}
    if not args.no_ts:
        t0 = time.monotonic()
        with ThreadPoolExecutor(args.jobs) as ex:
            for case, (ts, base) in zip(cases, ex.map(run_reference, cases)):
                refs[case['name']] = (ts, base)
        meta = next((r[0]['meta'] for r in refs.values() if r[0].get('meta')), None)
        print('reference: Node %s, V8 %s, %d fixtures, %.1f s' % ((meta or {}).get('node'), (meta or {}).get('v8'), len(cases), time.monotonic() - t0), flush=True)
        report['ts'] = {'meta': meta}
        bad = 0
        for case in cases:
            problems = check_expected(case, refs[case['name']][0])
            if problems:
                bad += 1
                failures.append(('ts', 'expected', case['name'], '; '.join(problems)))
                print('ts/expected: FAIL %s: %s' % (case['name'], problems[0]), flush=True)
        print('ts/expected: %d/%d fixtures do what they declare' % (len(cases) - bad, len(cases)), flush=True)
    seen_policies = {}
    for (compiler, backend), exe in exes.items():
        key = f'{compiler}/{backend}'
        side = f'{compiler}_{backend}'

        def one(case, exe=exe, side=side):
            impl = run_pair(case, side, exe)
            if args.no_ts:
                problems = check_expected(case, impl)
                return case, impl, {'ok': not problems, 'problems': problems, 'policies': [], 'notes': []}
            ts, base = refs[case['name']]
            return case, impl, judge(case, ts, base, impl)

        passed, t0 = 0, time.monotonic()
        with ThreadPoolExecutor(args.jobs) as ex:
            for case, impl, v in ex.map(one, cases):
                if v['ok']:
                    passed += 1
                    for p in v['policies']:
                        seen_policies.setdefault(p, set()).add(case['name'])
                    if not args.keep:
                        clean(impl['root'])
                    note = ' [%s]' % ','.join(v['policies']) if v['policies'] else ''
                    print(f'{key}: PASS {case["name"]}{note}', flush=True)
                else:
                    failures.append((compiler, backend, case['name'], '; '.join(v['problems'])))
                    print(f'{key}: FAIL {case["name"]}: {v["problems"][0]}', flush=True)
        report['pairs'][key] = {'passed': passed, 'of': len(cases), 'seconds': round(time.monotonic() - t0, 1)}
        print(f'{key}: {passed}/{len(cases)} passed', flush=True)
    report['policies_observed'] = {p: sorted(v) for p, v in sorted(seen_policies.items())}
    report['failures'] = failures
    if not args.no_ts:
        for case in cases:
            ts_run = refs[case['name']][0]
            if not args.keep and ts_run.get('root') and not any(f[2] == case['name'] for f in failures):
                clean(ts_run['root'])
    (WORK / 'test_impl_report.json').write_text(json.dumps(report, indent=1, default=str))
    (WORK / 'test_impl_failures.json').write_text(json.dumps(failures, indent=1))
    print('policies observed (a difference explained by the policy): ' + (', '.join('%s x%d' % (p, len(v)) for p, v in sorted(seen_policies.items())) or 'none'))
    print('FAILED: %d' % len(failures) if failures else 'all fixtures passed on %d pair(s)' % len(exes))
    return 1 if failures else 0


if __name__ == '__main__':
    raise SystemExit(main())
