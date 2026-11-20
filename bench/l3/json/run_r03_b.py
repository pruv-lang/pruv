#!/usr/bin/env python3
"""R03-B: verify and compile b_root.pruv (rs compiler, rust backend) and check that its query answers equal the
A4 oracle on run_a4's fixtures and seeded random documents (the same cases run_a4.py uses for a4_root)."""
import importlib.util
import json
from pathlib import Path
import shutil
import sys

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('a4', HERE / 'run_a4.py')
a4 = importlib.util.module_from_spec(spec); spec.loader.exec_module(a4)
WORK = HERE / 'work/r03_b'


def main():
    WORK.mkdir(parents=True, exist_ok=True)
    a4.a3.WORK = WORK
    g2 = a4.a3.configure()
    target = WORK / 'str-adapter'
    a4.run([shutil.which('cargo') or str(Path.home() / '.cargo/bin/cargo'), 'build', '--release', '--offline', '--quiet',
            '--manifest-path', HERE / 'str_adapter/Cargo.toml', '--target-dir', target])
    rlib, deps = target / 'release/libpruv_str.rlib', target / 'release/deps'
    flags = ['--rust-extern', f'pruv_str={rlib}', '--rust-search', str(deps), '--manifest', str(HERE / 'b_root.pruv.json')]
    rustflags = ['--extern', f'pruv_str={rlib}', '-L', f'dependency={deps}']
    cases = a4.fixtures()
    payload = ''.join('|'.join([a4.nums(d), a4.nums(o), a4.nums(k), a4.nums(t.encode()), str(m)]) + '\n' for d, o, k, t, m, w in cases)
    expected = [a4.nums(c[-1]) for c in cases]
    work = WORK / 'rs_rust'; work.mkdir(exist_ok=True)
    c = g2.compile_pruv('rs', str(HERE / 'b_root.pruv'), str(work), 'rust', flags)
    if c['rc'] != 0:
        print('compile/verify failed:', c['errors'] or c['stderr']); return 1
    print('rs/rust: B verified and compiled', flush=True)
    harness = work / 'main.rs'; harness.write_text(a4.HARNESS.replace('@SOL@', c['rs']))
    exe = work / 'runner'
    a4.run([g2.RUSTC, '-O', '--edition', '2021', harness, '-o', exe] + rustflags)
    got = a4.run([exe], payload).splitlines()
    bad = [i for i, (x, y) in enumerate(zip(expected, got)) if x != y]
    for i in bad[:5]: print(f'mismatch {i}: {cases[i]!r}; got={got[i]!r}')
    print(f'B cases={len(cases)} answers={len(got)} mismatches={len(bad)}')
    return 1 if bad or len(got) != len(expected) else 0


if __name__ == '__main__':
    sys.exit(main())
