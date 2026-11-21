"""L3-8 engine-import common entry: fixtures x four verified pairs (py/rust, py/llvm, rs/rust, rs/llvm).

For every fixture variant (I group excluded; integrate.py runs it) and each pair the runner (work/<pair>/harness, built from the
verified root.pruv, harness.rs and the entry_host crate) is compared
  - with the fixture's `expect` (terminal, value, trace; driver when recorded),
  - with the pinned-TypeScript oracle (including exact-limit probes),
  - across the pairs (all four identical).
The comparison classes are those of common.compare. Exit code is nonzero on any unexplained difference.

  python3 test_impl.py [--pairs py/rust,rs/llvm] [--only GROUP ...] [--case ID ...] [--reuse-verified] [--list]
"""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import subprocess
from pathlib import Path

from common import HERE, PAIRS, WORK, compare, host_path, load_cases, strip_case
from oracle import run_oracle

ROOT = HERE / 'root.pruv'
LOG = WORK / 'logs' / 'test_impl.log'
RUN_TIMEOUT = 3600


def tool(name):
    import shutil
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
    g, run = c.configure(WORK)
    return g


def build_host():
    """The entry_host crate (serde_json only; cargo locked + offline) -> (deps dir, newest rlib)."""
    target = WORK / 'host'
    sh([tool('cargo'), 'build', '--locked', '--offline', '--release', '--manifest-path', HERE / 'host/Cargo.toml', '--target-dir', target])
    deps = target / 'release/deps'
    libs = list(deps.glob('libentry_host-*.rlib'))
    if not libs:
        raise RuntimeError('no libentry_host rlib after the host build')
    return deps, max(libs, key=lambda p: p.stat().st_mtime)


def build_pair(g, compiler, backend, host, reuse=False):
    """Verify + compile root.pruv (manifest pruv.json, nothing passes via --no-verify), link harness.rs -> work/<pair>/harness."""
    out = WORK / f'{compiler}_{backend}'
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
    main.write_text((HERE / 'harness.rs').read_text(encoding='utf-8').replace('@SOL@', built['rs']), encoding='utf-8')
    extra = []
    if built.get('ll'):
        extra, err = g.build_llvm_objects(built['ll'], 'entry', str(out))
        if err:
            raise RuntimeError(err)
    exe = host_path(f'{compiler}_{backend}')
    sh([g.RUSTC, '-O', '--edition', '2021', main, '-o', exe, '--extern', f'entry_host={lib}', '-L', f'dependency={deps}'] + extra)
    print(f'{compiler}/{backend}: ' + ('reusing verified emission' if reuse else 'verified') + ', linked', flush=True)
    return exe


def run_runner(exe, cases):
    """One runner process over all cases (one JSON line each) -> list of result dicts (same order)."""
    payload = ''.join(json.dumps(strip_case_for_run(c), ensure_ascii=False).replace('\u2028', '\\u2028').replace('\u2029', '\\u2029').replace('\u0085', '\\u0085') + '\n' for c in cases)
    # probes are read by the host from "probe", so they stay in the line; only the fixture-only keys are dropped.
    p = subprocess.run([str(exe)], input=payload, capture_output=True, text=True, encoding='utf-8', timeout=RUN_TIMEOUT)
    lines = [ln for ln in p.stdout.split('\n') if ln.strip()]
    if p.returncode != 0 or len(lines) != len(cases):
        raise RuntimeError(f'runner exit {p.returncode}, {len(lines)} results for {len(cases)} cases: {p.stderr[-1500:]}')
    return [json.loads(ln) for ln in lines]


def strip_case_for_run(case):
    out = strip_case(case)
    if 'probe' in case:
        out['probe'] = case['probe']
    return out


def select(args):
    cases = [c for c in load_cases() if c['group'] != 'I']
    if args.only:
        wanted = {w.upper() for w in args.only}
        cases = [c for c in cases if c['group'] in wanted or c['parent'] in wanted]
    if args.case:
        wanted = set(args.case)
        cases = [c for c in cases if c['id'] in wanted or c['parent'] in wanted]
    return cases


def expect_line(case):
    exp = case['expect']
    return {'id': case['id'], 'terminal': exp['terminal'], 'value': exp['value'], 'trace': exp['trace'], 'driver': exp.get('driver')}


def judge(case, actual, oracle):
    """-> (class, field) of the actual result against expect and the oracle (the worse class wins)."""
    exp = expect_line(case)
    probe = case.get('probe')
    big = bool(probe) and probe['corrupt'] in ('oversize', 'oversize_phys')
    cls, field = compare(exp, actual, probe_too_large=big)
    if cls == 'exact' and exp.get('driver') is not None and actual.get('driver') != exp['driver']:
        cls, field = 'unexplained', 'driver'
    if cls == 'exact' and oracle is not None and (not probe or probe['corrupt'] in ('exact_limit', 'exact_limit_phys')):
        cls, field = compare(oracle, actual)
    if cls == 'exact' and 'callerTitles' in case['expect'] and 'callerTitles' in actual and actual['callerTitles'] != case['expect']['callerTitles']:
        cls, field = 'unexplained', 'callerTitles'
    return cls, field


def parse_pairs(text):
    if not text:
        return [tuple(p.split('_')) for p in PAIRS]
    out = []
    for p in text.split(','):
        c, b = p.strip().split('/')
        if f'{c}_{b}' not in PAIRS:
            raise SystemExit('unknown pair ' + p)
        out.append((c, b))
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--pairs', help='comma list of py/rust,py/llvm,rs/rust,rs/llvm (default all four)')
    ap.add_argument('--only', action='append', default=[], help='fixture group letter (S..P), repeatable')
    ap.add_argument('--case', action='append', default=[], help='one variant id (D01/list) or a fixture id (D01), repeatable')
    ap.add_argument('--reuse-verified', action='store_true', help='reuse work/<pair>/root_gen.* of an earlier verified run')
    ap.add_argument('--list', action='store_true', help='list the selected variants and exit')
    ap.add_argument('--show', type=int, default=12, help='how many differing cases to print per pair')
    args = ap.parse_args(argv)

    cases = select(args)
    if args.list:
        for c in cases:
            print(c['id'], c.get('name', ''))
        print(f'{len(cases)} variants')
        return 0
    if not cases:
        print('no fixture selected')
        return 2

    oracle = {r['id']: r for r in run_oracle([strip_case(c) for c in cases])}
    pairs = parse_pairs(args.pairs)
    g = configure()
    host = build_host()
    results = {}
    failed = False
    for compiler, backend in pairs:
        key = f'{compiler}/{backend}'
        try:
            exe = build_pair(g, compiler, backend, host, args.reuse_verified)
            results[key] = run_runner(exe, cases)
        except Exception as exc:
            failed = True
            print(f'{key}: ERROR {exc}', file=sys.stderr, flush=True)

    # per pair: expect / oracle comparison
    groups = sorted({c['group'] for c in cases})
    table = {}
    for key, got in results.items():
        shown = 0
        for case, actual in zip(cases, got):
            if actual.get('id') != case['id']:
                print(f'{key}: result id {actual.get("id")!r} for case {case["id"]}')
                failed = True
                continue
            cls, field = judge(case, actual, oracle[case['id']])
            row = table.setdefault((case['group'], key), {})
            row[cls] = row.get(cls, 0) + 1
            if cls == 'unexplained':
                failed = True
                if shown < args.show:
                    shown += 1
                    print(f'{key}: {case["id"]} differs at {field}: expected {json.dumps(expect_line(case), ensure_ascii=False)[:240]} got {json.dumps(actual, ensure_ascii=False)[:240]}')

    # across the pairs
    cross = {}
    keys = list(results)
    for i, case in enumerate(cases):
        base = results[keys[0]][i] if keys else None
        for key in keys[1:]:
            cls, field = compare(base, results[key][i])
            cross.setdefault(case['group'], {})
            cross[case['group']][cls] = cross[case['group']].get(cls, 0) + 1
            if cls == 'unexplained':
                failed = True
                print(f'cross-pair {keys[0]} vs {key}: {case["id"]} differs at {field}')

    print()
    print('group  ' + ''.join(f'{k:>34}' for k in results) + '   cross-pair')
    for grp in groups:
        cells = []
        for key in results:
            r = table.get((grp, key), {})
            cells.append(f'{r.get("exact", 0)} ok, {sum(v for c, v in r.items() if c != "exact")} other'.rjust(34))
        cr = cross.get(grp, {})
        print(f'{grp:<7}' + ''.join(cells) + '   ' + ', '.join(f'{c}={v}' for c, v in sorted(cr.items())))
    total = sum(len(v) for v in results.values())
    bad = sum(v.get('unexplained', 0) for r in table.values() for v in [r])
    print(f'{len(cases)} variants x {len(results)} pairs = {total} runs; unexplained differences: {bad}')
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
