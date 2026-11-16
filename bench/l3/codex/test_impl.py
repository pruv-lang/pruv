"""L3-5 Codex import: builds the four verified pairs (py/rust, py/llvm, rs/rust, rs/llvm) and runs the fixed fixtures
against the scripted fake app-server, comparing each pair with the checked-in expected JSON, with the pinned TypeScript
(Node, reference.mjs) and with the requests the fake recorded. Only this piece is built; no repository-wide suites.

    PATH="$HOME/.cargo/bin:$PATH" python3 bench/l3/codex/test_impl.py [--verify-only] [--reuse-verified] [--only SUBSTRING]
"""
import sys
sys.dont_write_bytecode = True
import importlib.util
import json
from pathlib import Path
from common import HERE, WORK, fake_binary, request_for, read_log, run_lines, node_command, check_case, load_cases

spec = importlib.util.spec_from_file_location('super_common', HERE.parent / 'supervisor/common.py')
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)


def build_host(g, run):
    """Host renderer crate (cargo, offline) and the string adapter; returns the extern flags and rustc extras."""
    run([str(Path.home() / '.cargo/bin/cargo'), 'build', '--offline', '--manifest-path', HERE / 'host/Cargo.toml', '--target-dir', WORK / 'host'])
    deps = WORK / 'host/debug/deps'
    lib = next(deps.glob('libcodex_host-*.rlib'))
    serde = next(deps.glob('libserde_json-*.rlib'))
    adapter = WORK / 'libpruv_str.rlib'
    run([g.RUSTC, '--crate-name', 'pruv_str', '--crate-type', 'rlib', HERE / 'str_adapter.rs', '-o', adapter])
    return deps, lib, serde, adapter


def build_pair(g, run, compiler, backend, flags, host, reuse=False):
    """Verify + compile root.pruv for one pair and build the harness executable (None with --verify-only)."""
    deps, lib, serde, adapter = host
    out = WORK / f'{compiler}_{backend}'
    out.mkdir(exist_ok=True)
    if reuse:
        built = {'rc': 0, 'rs': str(out / 'root_gen.rs'), 'll': str(out / 'root_gen.ll') if backend == 'llvm' else None}
        assert Path(built['rs']).is_file(), 'Run once without --reuse-verified first'
    else:
        built = g.compile_pruv(compiler, str(HERE / 'root.pruv'), str(out), backend, flags)
    if built['rc']:
        raise RuntimeError(str(built))
    print(f'{compiler}/{backend}: ' + ('reusing verified Pruv emission' if reuse else 'verified'), flush=True)
    harness = out / 'main.rs'
    harness.write_text((HERE / 'harness.rs').read_text().replace('@SOL@', built['rs']))
    extra = []
    if built['ll']:
        extra, err = g.build_llvm_objects(built['ll'], 'codex', str(out))
        assert not err, err
    exe = out / 'runner'
    run([g.RUSTC, '-O', '--edition', '2021', harness, '-o', exe, '--extern', f'codex_host={lib}', '--extern', f'pruv_str={adapter}',
         '--extern', f'serde_json={serde}', '-L', f'dependency={deps}', *extra])
    return exe


def main():
    g, run = c.configure(WORK)
    only = sys.argv[sys.argv.index('--only') + 1] if '--only' in sys.argv else None
    host = build_host(g, run)
    flags = ['--rust-extern', f'pruv_str={host[3]}', '--manifest', str(HERE / 'pruv.json')]
    binary = fake_binary()
    cases = [x for x in load_cases() if not only or only in x['name']]
    failures = []
    # pinned TypeScript first: it must agree with the checked-in expected JSON, and it is the comparison for every pair
    ts_requests = [request_for(case, 'ts', i, binary) for i, case in enumerate(cases)]
    ts_results = [] if '--verify-only' in sys.argv else run_lines(node_command(), ts_requests)
    ts_logs = [read_log(i, 'ts') for i in range(len(cases))]
    passed = 0
    for i, (case, result) in enumerate(zip(cases, ts_results)):
        problems = check_case(case, result, ts_logs[i])
        if problems:
            failures.append(('ts', 'oracle', case['name'], '; '.join(problems)))
            print(f'ts/oracle: FAIL {case["name"]}: {problems[0]}', flush=True)
        else:
            passed += 1
    if ts_results:
        print(f'ts/oracle: {passed}/{len(cases)} passed (pinned TypeScript against the expected JSON)', flush=True)
    for compiler, backend in c.PAIRS:
        key = f'{compiler}/{backend}'
        exe = build_pair(g, run, compiler, backend, flags, host, '--reuse-verified' in sys.argv)
        if '--verify-only' in sys.argv:
            continue
        requests = [request_for(case, f'{compiler}_{backend}', i, binary) for i, case in enumerate(cases)]
        results = run_lines([exe], requests)
        passed = 0
        for i, case in enumerate(cases):
            problems = check_case(case, results[i], read_log(i, f'{compiler}_{backend}'), ts_results[i], ts_logs[i])
            if problems:
                failures.append((compiler, backend, case['name'], '; '.join(problems)))
                print(f'{key}: FAIL {case["name"]}: {problems[0]}', flush=True)
            else:
                passed += 1
        print(f'{key}: {passed}/{len(cases)} passed', flush=True)
    (WORK / 'test_impl_failures.json').write_text(json.dumps(failures, indent=1))
    if failures:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
