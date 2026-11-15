"""G10 proc benchmark; all generated files stay under bench/g10_proc/.work.

PRUV_PYTHON, PRUVC_RS, RUSTC and CLANG can select existing tools without building/installing.
"""
import sys
sys.dont_write_bytecode = True
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
WORK = HERE / '.work'
WORK.mkdir(exist_ok=True)
os.environ['TMPDIR'] = str(WORK)
os.environ['PYTHONDONTWRITEBYTECODE'] = '1'
os.environ['CLANG_MODULE_CACHE_PATH'] = str(WORK / 'clang-cache')
tempfile.tempdir = str(WORK)
spec = importlib.util.spec_from_file_location('g2_run', ROOT / 'bench/g2/run.py')
G2 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(G2)
G2.COMPILERS['py'] = [os.environ.get('PRUV_PYTHON', str(ROOT / '.venv/bin/python')
                         if (ROOT / '.venv/bin/python').exists() else sys.executable), str(ROOT / 'pruvc/pruvc.py')]
G2.COMPILERS['rs'] = [os.environ.get('PRUVC_RS', str(ROOT / 'pruvc-rs/target/release/pruvc'))]
G2.RUSTC = os.environ.get('RUSTC', G2.RUSTC)
G2.CLANG = os.environ.get('CLANG', G2.CLANG)
PROBLEMS = ['q01', 'q02']
HEADERS = {
    'q01': 'pub fn version(program: str, flag: str, cwd: str) -> i64 ! proc, alloc '
           'ensures -31 <= result && result < 256000000000000',
    'q02': 'pub fn scan(program: str, mode: str, budget: i64) -> i64 ! proc, alloc '
           'requires 0 <= budget && budget <= 10000 ensures -4 <= result && result <= 4096000040960000',
}


def run(cmd, timeout=180, env=None, cwd=None):
    try:
        r = subprocess.run([str(x) for x in cmd], capture_output=True, text=True, timeout=timeout,
                           env=env, cwd=cwd, errors='replace')
        return {'rc': r.returncode, 'stdout': r.stdout, 'stderr': r.stderr}
    except subprocess.TimeoutExpired:
        return {'rc': -1, 'stdout': '', 'stderr': 'timeout'}
    except OSError as e:
        return {'rc': -2, 'stdout': '', 'stderr': str(e)}


def child_lines(kind):
    """The stdout text of `child lines KIND` (child.rs keeps the same definitions)."""
    if kind in ('empty', 'forever'):
        return b''
    if kind == 'one':
        return b'abc'
    if kind == 'blank':
        return b'\n\n\n'
    if kind == 'long':
        return b'x' * 20000 + b'\nyz'
    if kind == 'mix':
        return b'\n'.join(bytes([97 + i % 26]) * ((i * 37) % 5000) for i in range(300))
    if kind == 'bin':
        return bytes(b for b in range(256) if b != 10) + b'\n\r\n\r'
    if kind == 'big':
        return (b'q' * 999 + b'\n') * 3000
    if kind == 'fail':
        return b'partial\n'
    raise ValueError(kind)


def ref_scan(kind, budget):
    """Python oracle for q02 with the fixture child; None for the spawn-failure case."""
    if kind in ('fail',):
        return -4
    data = child_lines(kind)
    reads = (len(data) + 4095) // 4096 + 1      # lower bound on the reads to reach EOF; fixtures stay far from it
    if kind == 'forever' or budget < reads:
        return -3
    parts = data.split(b'\n')
    if parts and parts[-1] == b'':
        parts.pop()
    return len(parts) * 100000000 + max([len(x) for x in parts], default=0)


# (kind, budget); budget far above or below the reads each fixture needs, so chunking never decides a case
SCAN_CASES = [('empty', 10), ('empty', 1), ('empty', 0), ('one', 10), ('one', 0), ('blank', 10), ('long', 50),
              ('mix', 1000), ('bin', 10), ('big', 10000), ('big', 10), ('fail', 100), ('forever', 50),
              ('forever', 0)]
FLAGS = ['', 'abc', 'a b', 'héllo 日本', 'quote"in\\side', "-x=1 'y'", 'z' * 255, 'w' * 300]


def driver(pid):
    common = '''fn check(p: &mut usize, t: &mut usize, name: &str, ok: bool) {
        *t += 1; if ok { *p += 1; } else { println!("FAIL {}", name); }
    }
    '''
    lines = ['let child = std::env::var("PRUV_PROC_CHILD").unwrap();',
             'let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap());',
             'std::fs::create_dir_all(&dir).unwrap();', 'let mut passed = 0; let mut total = 0;']
    if pid == 'q01':
        common += '''fn h(b: &[u8]) -> i64 { let mut h = 1i64; for &x in b { h = (h * 256 + x as i64) % 999983; } h }
    fn want(flag: &str, cwd: &std::path::Path) -> i64 {
        let out = format!("--version\\n{}\\n", flag);
        let err = format!("{}\\n", cwd.to_string_lossy());
        (flag.len() % 256) as i64 * 1000000000000 + h(out.as_bytes()) * 1000000 + h(err.as_bytes())
    }
    '''
        lines += ['let here = std::env::current_dir().unwrap().canonicalize().unwrap();',
                  'let sub = dir.join("sub dir \\u{65e5}"); std::fs::create_dir_all(&sub).unwrap();',
                  'let subc = sub.canonicalize().unwrap();']
        for i, f in enumerate(FLAGS):
            lit = json.dumps(f, ensure_ascii=False)
            lines += [f'check(&mut passed, &mut total, "flag_{i}", sol::version(&child, {lit}, "") == want({lit}, &here));',
                      f'check(&mut passed, &mut total, "cwd_{i}", sol::version(&child, {lit}, sub.to_str().unwrap()) == want({lit}, &subc));']
        lines += ['check(&mut passed, &mut total, "empty_program", sol::version("", "a", "") == -11);',
                  'let missing = dir.join("no_such_program");',
                  'check(&mut passed, &mut total, "missing_program", sol::version(missing.to_str().unwrap(), "a", "") == -12);',
                  'let nocwd = dir.join("no_such_dir");',
                  'check(&mut passed, &mut total, "missing_cwd", sol::version(&child, "a", nocwd.to_str().unwrap()) == -12);',
                  'check(&mut passed, &mut total, "signal_exit", sol::version(&child, "ABORT", "") == -31);']
    else:
        for kind, budget in SCAN_CASES:
            lines += [f'check(&mut passed, &mut total, "{kind}_{budget}", sol::scan(&child, "{kind}", {budget}) == {ref_scan(kind, budget)});']
        lines += ['let missing = dir.join("no_such_program");',
                  'check(&mut passed, &mut total, "missing_program", sol::scan(missing.to_str().unwrap(), "one", 10) == -1);',
                  'check(&mut passed, &mut total, "unknown_mode", sol::scan(&child, "nope", 10) == -4);']
    lines += ['println!("TESTS {} {}", passed, total);']
    return common + 'fn main() {' + '\n'.join(lines) + '\n}'


def child_exe():
    exe = WORK / 'child'
    src = HERE / 'child.rs'
    if not exe.exists() or exe.stat().st_mtime < src.stat().st_mtime:
        r = run([G2.RUSTC, '-O', '--edition=2021', src, '-o', exe])
        if r['rc']:
            raise SystemExit('child build failed: ' + r['stderr'][-800:])
    return exe


def build(c, wd, body):
    extra = []
    if c['ll']:
        try:
            extra, error = G2.build_llvm_objects(c['ll'], 'g10p', str(wd))
        except OSError as e:
            return None, str(e)
        if error:
            return None, error
    main, exe = wd / 'main.rs', wd / 'program'
    main.write_text('#![allow(dead_code,unused_imports,unused_parens)]\n#[path = ' + json.dumps(c['rs']) + '] mod sol;\n' + body)
    result = run([G2.RUSTC, '-O', '--edition=2021', main, '-o', exe] + extra)
    return (exe, None) if result['rc'] == 0 else (None, result['stderr'])


def check_header(src, pid):
    text = re.sub(r'//[^\n]*', '', src.read_text())
    m = re.search(r'pub\s+fn\s+\w+[^\{]*', text)
    return m and ' '.join(m.group().split()) == HEADERS[pid]


def grade_one(src, pid, compiler, backend, flags=()):
    src = Path(src).resolve()
    if not src.exists():
        return {'status': 'missing'}
    if not check_header(src, pid):
        return {'status': 'harness_error', 'error': 'contract changed'}
    child = child_exe()
    with tempfile.TemporaryDirectory(prefix='grade_', dir=WORK) as tmp:
        wd = Path(tmp)
        c = G2.compile_pruv(compiler, str(src), str(wd), backend, flags)
        if c['rc']:
            return dict(c, status='pruv_error')
        exe, error = build(c, wd, driver(pid))
        if error:
            return {'status': 'compile_error', 'error': error}
        r = run([exe, wd / 'fixtures'], timeout=120, env=dict(os.environ, PRUV_PROC_CHILD=str(child)))
        m = re.search(r'TESTS (\d+) (\d+)', r['stdout'])
        passed, total = map(int, m.groups()) if m else (0, -1)
        return dict(r, status='ok' if r['rc'] == 0 and passed == total else 'wrong',
                    tests_passed=passed, tests_total=total)


def main():
    p = argparse.ArgumentParser(__doc__)
    p.add_argument('solutions', type=Path)
    p.add_argument('out', nargs='?', type=Path)
    p.add_argument('--compiler', choices=['py', 'rs', 'both'], default='both')
    p.add_argument('--backend', choices=['rust', 'llvm', 'both'], default='both')
    p.add_argument('--no-verify', action='store_true')
    a = p.parse_args()
    results = {}
    for comp in ['py', 'rs'] if a.compiler == 'both' else [a.compiler]:
        for backend in ['rust', 'llvm'] if a.backend == 'both' else [a.backend]:
            key = comp + '-' + backend
            results[key] = {}
            for pid in PROBLEMS:
                r = grade_one(a.solutions / pid / 'root.pruv', pid, comp, backend,
                              ['--no-verify'] if a.no_verify else [])
                results[key][pid] = r
                print(key, pid, r['status'], f"{r.get('tests_passed', '-')}/{r.get('tests_total', '-')}", flush=True)
                if r['status'] != 'ok':
                    print(r.get('stdout', '')[-600:], r.get('stderr') or r.get('error') or r.get('errors'))
    out = a.out or HERE / 'results' / f'solutions_{a.compiler}_{a.backend}.json'
    out.parent.mkdir(exist_ok=True)
    out.write_text(json.dumps({'tools': G2.COMPILERS, 'results': results}, indent=2))
    return int(any(r['status'] != 'ok' for rows in results.values() for r in rows.values()))


if __name__ == '__main__':
    sys.exit(main())
