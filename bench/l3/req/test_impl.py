"""L3-9 engine-import request handlers: 160 fixtures x the verified pairs (default rs/rust, rs/llvm; py/rust, py/llvm only via --pairs).

For each pair the runner builds work/build/<pair>/harness from bench/l3/req/root.pruv (compiled and verified with --lib
bench/l3/lib), the req_host crate (bench/l3/req/host) and harness.rs. Every fixture scenario then goes through the pinned-TypeScript
oracle and through each pair harness (one process per pair; JSON lines in, one observation line per scenario out). Each result is
classified by compare.py (exact / P5 / P6 / engine-policy / excluded-domain / unexplained; P08 through the explicit 32-active-request
P6 policy), and the selected pairs are compared with each other (must be identical). Exit code nonzero on any unexplained difference.
First differences are saved in work/test_impl/<pair>/<id>.json and work/test_impl/summary.json.

  python3 test_impl.py [--pairs py/rust,rs/llvm] [--group S,F | --only S ...] [--id P08 ...] [--list]
                       [--oracle-only] [--reuse-verified] [--show 12]

--oracle-only runs just the oracle side (determinism, driver, pending checks) and needs neither root.pruv nor host/.
"""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import shutil
import subprocess
from collections import Counter
from pathlib import Path

import compare
from common import GROUPS, HERE, load_all, strip_case
from oracle import run_oracle

# PF: the Python compiler pruvc/ is frozen (bundle PF); the Rust compiler is the product. Default pairs are rs only;
# ALL_PAIRS keeps the py pairs selectable with --pairs py/rust,... as the frozen reference (not used by CI).
ALL_PAIRS = ('py/rust', 'py/llvm', 'rs/rust', 'rs/llvm')
PAIRS = ('rs/rust', 'rs/llvm')
ROOT = HERE / 'root.pruv'
HOST = HERE / 'host'
HARNESS = HERE / 'harness.rs'
WORK = HERE / 'work'
OUT = WORK / 'test_impl'
BUILD = WORK / 'build'
LOG = OUT / 'logs' / 'test_impl.log'
RUN_TIMEOUT = 3600


def group_of(fixture):
    return fixture['id'][0]


def tool(name):
    return shutil.which(name) or str(Path.home() / '.cargo/bin' / name)


def sh(cmd, timeout=3600):
    cmd = [str(x) for x in cmd]
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    LOG.parent.mkdir(parents=True, exist_ok=True)
    with open(LOG, 'a', encoding='utf-8') as f:
        f.write('$ %s\n[exit %d]\n%s%s\n' % (' '.join(cmd), p.returncode, p.stdout, p.stderr))
    if p.returncode:
        raise RuntimeError('%s: exit %d\n%s\n%s' % (' '.join(cmd[:4]), p.returncode, p.stdout[-1500:], p.stderr[-2500:]))
    return p


def configure():
    """The shared compiler driver (supervisor common -> g with compile_pruv / build_llvm_objects / RUSTC)."""
    import importlib.util
    spec = importlib.util.spec_from_file_location('super_common', HERE.parent / 'supervisor/common.py')
    c = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(c)
    g, run = c.configure(BUILD)
    return g


def missing_prerequisites():
    return [str(p.relative_to(HERE.parent.parent.parent)) for p in (ROOT, HOST / 'Cargo.toml', HARNESS) if not p.exists()]


def build_host():
    """The req_host crate (cargo locked + offline) -> (deps dir, newest rlib)."""
    target = BUILD / 'host'
    sh([tool('cargo'), 'build', '--locked', '--offline', '--release', '--manifest-path', HOST / 'Cargo.toml', '--target-dir', target])
    deps = target / 'release/deps'
    libs = list(deps.glob('libreq_host-*.rlib'))
    if not libs:
        raise RuntimeError('no libreq_host rlib after the host build')
    return deps, max(libs, key=lambda p: p.stat().st_mtime)


def pair_dir(pair):
    return BUILD / pair.replace('/', '_')


def build_pair(g, pair, host, reuse=False):
    """Verify + compile root.pruv, link harness.rs -> work/build/<pair>/harness."""
    compiler, backend = pair.split('/')
    out = pair_dir(pair)
    out.mkdir(parents=True, exist_ok=True)
    if reuse:
        built = {'rc': 0, 'rs': str(out / 'root_gen.rs'), 'll': str(out / 'root_gen.ll') if backend == 'llvm' else None}
        if not Path(built['rs']).is_file():
            raise RuntimeError('run once without --reuse-verified first')
    else:
        built = g.compile_pruv(compiler, str(ROOT), str(out), backend, ['--lib', str(HERE.parent / 'lib')])
    if built['rc']:
        raise RuntimeError('compile/verify failed: ' + str(built.get('errors') or built.get('stderr') or built))
    deps, lib = host
    main = out / 'main.rs'
    main.write_text(HARNESS.read_text(encoding='utf-8').replace('@SOL@', built['rs']), encoding='utf-8')
    extra = []
    if built.get('ll'):
        extra, err = g.build_llvm_objects(built['ll'], 'req', str(out))
        if err:
            raise RuntimeError(err)
    exe = out / 'harness'
    sh([g.RUSTC, '-O', '--edition', '2021', main, '-o', exe, '--extern', f'req_host={lib}', '-L', f'dependency={deps}'] + extra)
    print(f'{pair}: ' + ('reusing verified emission' if reuse else 'verified') + ', linked', flush=True)
    return exe


def run_runner(exe, cases):
    """One runner process over all cases -> list of result lines (str, order kept) or one exception for a crash."""
    payload = ''.join(json.dumps(strip_case(c), ensure_ascii=False).replace('\u2028', '\\u2028').replace('\u2029', '\\u2029').replace('\u0085', '\\u0085') + '\n' for c in cases)
    try:
        p = subprocess.run([str(exe)], input=payload, capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=RUN_TIMEOUT)
    except Exception as exc:
        return RuntimeError(f'runner failed to run: {exc}')
    lines = [ln for ln in p.stdout.split('\n') if ln.strip()]
    if p.returncode != 0:
        # keep what was produced; the rest is "crash" in the batch comparison
        err = RuntimeError(f'runner exit {p.returncode}: {p.stderr[-800:]}')
        return lines + [err] * max(0, len(cases) - len(lines))
    return lines


def select(args):
    cases = load_all()
    groups = set()
    for g in args.group + args.only:
        groups.update(x.strip().upper() for x in g.split(',') if x.strip())
    bad = groups - set(GROUPS)
    if bad:
        raise SystemExit('unknown group(s): ' + ','.join(sorted(bad)))
    if groups:
        cases = [c for c in cases if group_of(c) in groups]
    if args.id:
        wanted = set(args.id)
        unknown = wanted - {c['id'] for c in cases}
        if unknown:
            raise SystemExit('unknown or filtered-out id(s): ' + ','.join(sorted(unknown)))
        cases = [c for c in cases if c['id'] in wanted]
    return cases


def parse_pairs(text):
    if not text:
        return list(PAIRS)
    out = []
    for p in text.split(','):
        p = p.strip()
        if p not in ALL_PAIRS:
            raise SystemExit('unknown pair %s (use %s)' % (p, ', '.join(ALL_PAIRS)))
        out.append(p)
    return out


def oracle_problems(case, obs):
    import q
    return q.check_oracle(case, obs)


def save_diff(pair, case, expected, actual, res):
    d = OUT / pair.replace('/', '_')
    d.mkdir(parents=True, exist_ok=True)
    (d / f"{case['id']}.json").write_text(json.dumps({'id': case['id'], 'pair': pair, 'result': res, 'expected': expected, 'actual': actual}, ensure_ascii=False, indent=1), encoding='utf-8')


def clear_saved():
    if OUT.exists():
        for d in OUT.iterdir():
            if d.is_dir() and d.name != 'logs':
                shutil.rmtree(d)


def short(v, n=200):
    return json.dumps(v, ensure_ascii=False)[:n]


def table(rows, keys, label):
    classes = compare.CLASSES
    print(f'{label:<6}' + ''.join(f'{k:>12}' for k in keys))
    for grp, cells in sorted(rows.items()):
        line = f'{grp:<6}'
        for k in keys:
            c = cells.get(k, Counter())
            other = sum(v for cl, v in c.items() if cl != 'exact')
            line += f'{str(c.get("exact", 0)) + "/" + str(other):>12}'
        print(line)
    print('(cells: exact/non-exact; non-exact classes: ' + ', '.join(classes[1:]) + ')')


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--pairs', help='comma list of ' + ','.join(ALL_PAIRS) + ' (default ' + ','.join(PAIRS) + '; py is the frozen PF reference)')
    ap.add_argument('--group', action='append', default=[], help='fixture group letters, comma list or repeated (S F B L A O T U G R E C D I P X)')
    ap.add_argument('--only', action='append', default=[], help='same as --group (entry/test_impl.py spelling)')
    ap.add_argument('--id', action='append', default=[], help='one fixture id (P08), repeatable')
    ap.add_argument('--case', action='append', default=[], dest='id', help='alias of --id')
    ap.add_argument('--reuse-verified', action='store_true', help='reuse work/build/<pair>/root_gen.* of an earlier verified run')
    ap.add_argument('--oracle-only', action='store_true', help='run only the oracle side (no root.pruv / host needed)')
    ap.add_argument('--list', action='store_true', help='list the selected fixtures and exit')
    ap.add_argument('--show', type=int, default=12, help='how many differing cases to print per pair')
    args = ap.parse_args(argv)

    cases = select(args)
    pairs = parse_pairs(args.pairs)
    if args.list:
        for c in cases:
            print(c['id'], c.get('name', ''))
        print(f'{len(cases)} fixtures')
        return 0
    if not cases:
        print('no fixture selected')
        return 2
    if not args.oracle_only:
        missing = missing_prerequisites()
        if missing:
            print('cannot run the implementation pairs yet: missing ' + ', '.join(missing) +
                  ' (the Pruv core and Rust host are not built). Use --oracle-only to check the oracle side.', file=sys.stderr)
            return 2

    oracle = run_oracle(cases)
    again = run_oracle(cases)
    failed = False
    nondet = [c['id'] for c, a, b in zip(cases, oracle, again) if a != b]
    problems = [(c['id'], p) for c, o in zip(cases, oracle) for p in oracle_problems(c, o)]
    batch0 = compare.compare_batch(oracle, oracle, policies=lambda i, e: None)
    print(f'oracle: {len(cases)} fixtures, nondeterministic: {len(nondet)}, driver/pending problems: {len(problems)}, self-compare exact: {batch0["counts"]["exact"]}')
    for i in nondet[:10]:
        print('  nondeterministic oracle result:', i)
    for i, p in problems[:10]:
        print('  oracle problem', i, p[:200])
    if nondet or problems or not batch0['ok']:
        failed = True
    if args.oracle_only:
        return 1 if failed else 0

    clear_saved()
    runs = {}
    try:
        g = configure()
        host = build_host()
    except Exception as exc:
        print(f'build ERROR {exc}', file=sys.stderr, flush=True)
        return 1
    for pair in pairs:
        try:
            exe = build_pair(g, pair, host, args.reuse_verified)
            runs[pair] = run_runner(exe, cases)
        except Exception as exc:
            failed = True
            print(f'{pair}: ERROR {exc}', file=sys.stderr, flush=True)

    rows = {}
    unexplained = 0
    summary = {'pairs': {}, 'crossPair': []}
    parsed = {}
    for pair, lines in runs.items():
        batch = compare.compare_batch(oracle, lines)
        shown = 0
        parsed[pair] = [None] * len(cases)
        for i, (case, res) in enumerate(zip(cases, batch['results'])):
            rows.setdefault(group_of(case), {}).setdefault(pair, Counter())[res['class']] += 1
            if isinstance(lines, list) and i < len(lines) and isinstance(lines[i], str):
                try:
                    parsed[pair][i] = json.loads(lines[i])
                except ValueError:
                    pass
            if res['class'] == 'unexplained':
                unexplained += 1
                save_diff(pair, case, oracle[i], parsed[pair][i], res)
                if shown < args.show:
                    shown += 1
                    why = res.get('reason') or 'differs'
                    print(f"{pair}: {case['id']} {why} at {res['path']}: expected {short(res['expected'])} got {short(res['actual'])}")
        for res in batch['results'][len(cases):]:
            unexplained += 1
            print(f"{pair}: extra result {res.get('id')}")
        summary['pairs'][pair] = {'counts': batch['counts'], 'first': [r for r in batch['results'] if r['class'] != 'exact'][:50]}

    # the selected pairs must agree with each other exactly (same policy-free comparison)
    keys = list(parsed)
    for i, case in enumerate(cases):
        base = parsed[keys[0]][i] if keys else None
        for other in keys[1:]:
            cur = parsed[other][i]
            if base is None or cur is None:
                continue  # a missing/malformed side was already counted above
            r = compare.compare_observation(base, cur)
            if r['class'] != 'exact':
                unexplained += 1
                summary['crossPair'].append({'id': case['id'], 'a': keys[0], 'b': other, 'path': r['path']})
                print(f"cross-pair {keys[0]} vs {other}: {case['id']} differs at {r['path']}")

    print()
    table(rows, list(runs), 'group')
    total = sum(len(cases) for _ in runs)
    print(f'{len(cases)} variants x {len(runs)} pairs = {total} runs; unexplained differences: {unexplained}')
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / 'summary.json').write_text(json.dumps(summary, ensure_ascii=False, indent=1), encoding='utf-8')
    if len(runs) != len(pairs):
        failed = True
    return 1 if failed or unexplained else 0


if __name__ == '__main__':
    sys.exit(main())
