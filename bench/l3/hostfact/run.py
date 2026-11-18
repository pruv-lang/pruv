"""P10H runner: host rlib + root.pruv on py/rs x rust/llvm, Rust harness `gates` and `perf`.
usage: run.py [--rounds N] [--only gates|perf]   (work files under bench/l3/hostfact/work/)"""
import sys
sys.dont_write_bytecode = True
import argparse, importlib.util, json, os, subprocess
from pathlib import Path
HERE = Path(__file__).resolve().parent
WORK = HERE / 'work'
spec = importlib.util.spec_from_file_location('super_common', HERE.parent / 'supervisor/common.py')
c = importlib.util.module_from_spec(spec); spec.loader.exec_module(c)

ap = argparse.ArgumentParser(description=__doc__)
ap.add_argument('--rounds', type=int, default=7)
ap.add_argument('--only', choices=['gates', 'perf'])
args = ap.parse_args()
if args.rounds < 7:
    ap.error('--rounds must be >= 7')

g, run = c.configure(WORK)


def sh(cmd, data=None):
    print('+ ' + ' '.join(str(x) for x in cmd), flush=True)
    return run(cmd, data)


# host rlib: same flags as bench/g6/run.py build_rlib
rlib = WORK / 'libhostfact_host.rlib'
sh([g.RUSTC, '-O', '--edition', '2021', '--crate-type', 'rlib', '--crate-name', 'hostfact_host', HERE / 'host.rs', '-o', rlib])
# flags as bench/l3/mcp/test_impl.py
flags = ['--rust-extern', f'hostfact_host={rlib}', '--manifest', str(HERE / 'pruv.json')]

failed = False
perf = {}
for compiler, backend in c.PAIRS:
    key = f'{compiler}/{backend}'
    try:
        out = WORK / f'{compiler}_{backend}'; out.mkdir(exist_ok=True)
        built = g.compile_pruv(compiler, str(HERE / 'root.pruv'), str(out), backend, flags)
        if built['rc']:
            raise RuntimeError('compile/verify failed: ' + str(built['errors'] or built['stderr']))
        print(f'{key}: verified and compiled', flush=True)
        harness = out / 'main.rs'
        harness.write_text((HERE / 'harness.rs').read_text().replace('@SOL@', built['rs']))
        extra = []
        if built['ll']:
            extra, err = g.build_llvm_objects(built['ll'], 'hostfact', str(out))
            if err:
                raise RuntimeError(err)
        exe = out / 'runner'
        sh([g.RUSTC, '-O', '--edition', '2021', harness, '-o', exe, '--extern', f'hostfact_host={rlib}', *extra])
        if args.only != 'perf':
            print('+ ' + str(exe) + ' gates', flush=True)
            p = subprocess.run([str(exe), 'gates'], capture_output=True, text=True, timeout=600)
            for line in p.stdout.splitlines():
                print(f'{key}: {line}')
            if p.returncode:
                failed = True
                print(f'{key}: gates exit {p.returncode}\n{p.stderr[-500:]}', file=sys.stderr)
        if args.only != 'gates':
            print('+ ' + f'{exe} perf {args.rounds} {compiler}_{backend}', flush=True)
            p = subprocess.run([str(exe), 'perf', str(args.rounds), f'{compiler}_{backend}'], capture_output=True, text=True, timeout=3600)
            if p.returncode:
                raise RuntimeError(f'perf exit {p.returncode}\n{p.stdout}\n{p.stderr[-500:]}')
            for line in p.stdout.splitlines():
                print(line)
                if line.startswith('{'):
                    j = json.loads(line)
                    perf.setdefault(key, {})[j['lane']] = j
    except Exception as exc:
        failed = True
        print(f'{key}: ERROR {exc}', file=sys.stderr, flush=True)

if perf:
    print('\nperf summary (median ns over %d rounds; ratio = Pruv / hand)' % args.rounds)
    print('%-9s %-22s %12s %12s %12s %7s %9s %9s' % ('build', 'lane', 'pruv', 'hand', 'control', 'ratio', 'pruv/call', 'hand/call'))
    for key, lanes in perf.items():
        for name, pl, hl, cl in (('rq_find 20000x10', 'rq_find_pruv', 'rq_find_hand', 'rq_find_control'),
                                 ('bh_halt x200000', 'halt_pruv', 'halt_hand', None)):
            if pl in lanes and hl in lanes:
                p, h = lanes[pl], lanes[hl]
                ctl = lanes[cl]['median_ns'] if cl and cl in lanes else '-'
                print('%-9s %-22s %12d %12d %12s %7.3f %9.2f %9.2f' % (
                    key, name, p['median_ns'], h['median_ns'], ctl, p['median_ns'] / max(h['median_ns'], 1),
                    p['per_call_ns'], h['per_call_ns']))
sys.exit(1 if failed else 0)
