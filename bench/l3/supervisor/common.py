"""Shared driver for the L3-2 batch tests: compile a root on py/rs x rust/llvm and run a Rust harness."""
import importlib.util
from itertools import groupby
import os
from pathlib import Path
import sys

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
PAIRS = [(c, b) for c in ('py', 'rs') for b in ('rust', 'llvm')]


def enc(values):
    """Run-length word encoding of an int sequence; `v*k` repeats v k times."""
    words = []
    for v, grp in groupby(values):
        k = len(list(grp))
        words.append(f'{v}*{k}' if k >= 4 else ' '.join([str(v)] * k))
    return ' '.join(w for w in words if w)


EXPAND = r'''
fn expand<'a>(words: impl Iterator<Item = &'a str>) -> Vec<i64> {
 let mut out = Vec::new();
 for w in words {
  match w.split_once('*') {
   Some((v, k)) => { let v: i64 = v.parse().unwrap(); for _ in 0..k.parse::<usize>().unwrap() { out.push(v); } }
   None => out.push(w.parse().unwrap()),
  }
 }
 out
}
fn bytes(v: &[i64]) -> Vec<u8> { v.iter().map(|&x| x as u8).collect() }
fn text(v: &[i64]) -> String { String::from_utf8(bytes(v)).unwrap() }
fn show(out: &[i64]) -> String { out.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ") }
'''


def configure(work):
    work.mkdir(parents=True, exist_ok=True)
    os.environ['TMPDIR'] = str(work)
    spec = importlib.util.spec_from_file_location('a2_setup', HERE.parent / 'json' / 'run_a2.py')
    setup = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(setup)
    setup.WORK = work
    return setup.configure(), setup.run


def build(g2, run, compiler, backend, root, work, harness_src, name):
    """Verify and compile root, build the harness, return the executable path."""
    work.mkdir(parents=True, exist_ok=True)
    compiled = g2.compile_pruv(compiler, str(root), str(work), backend)
    if compiled['rc'] != 0:
        raise RuntimeError('compile/verify failed: ' + str(compiled['errors'] or compiled['stderr']))
    harness = work / 'main.rs'
    harness.write_text(harness_src.replace('@SOL@', compiled['rs']))
    extra = []
    if compiled['ll']:
        extra, error = g2.build_llvm_objects(compiled['ll'], name, str(work))
        if error:
            raise RuntimeError(error)
    exe = work / 'runner'
    run([g2.RUSTC, '-O', '--edition', '2021', harness, '-o', exe] + extra)
    return exe


def run_lines(name, root, harness_src, cases, pairs=PAIRS):
    """cases: list of (label, input line, expected output line). Returns the process exit code."""
    work_root = HERE / 'work' / name
    g2, run = configure(work_root)
    payload = ''.join(line + '\n' for _, line, _ in cases)
    failed = False
    for compiler, backend in pairs:
        key = f'{compiler}/{backend}'
        try:
            exe = build(g2, run, compiler, backend, root, work_root / f'{compiler}_{backend}', harness_src, name)
            print(f'{key}: verified and compiled', flush=True)
            got = run([exe], payload).splitlines()
            if len(got) != len(cases):
                raise RuntimeError(f'{len(got)} answers for {len(cases)} fixtures')
            bad = [(i, w, g) for i, ((_, _, w), g) in enumerate(zip(cases, got)) if w != g]
            for i, w, g in bad[:20]:
                print(f'{key}: fixture {i} {cases[i][0]}: expected {w[:200]}, got {g[:200]}')
            failed |= bool(bad)
            print(f'{key}: {len(cases) - len(bad)}/{len(cases)} fixtures passed', flush=True)
        except Exception as exc:
            failed = True
            print(f'{key}: ERROR {exc}', file=sys.stderr, flush=True)
    return int(failed)
