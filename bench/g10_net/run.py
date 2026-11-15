"""G10 net benchmark; all generated files stay under bench/g10_net/.work.

PRUV_PYTHON, PRUVC_RS, RUSTC and CLANG can select existing tools without building/installing.
The Rust driver hosts every peer itself in threads on 127.0.0.1; there is no child process.
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
    'q01': 'pub fn call(host: str, port: i64, request: [u8]) -> i64 ! net, alloc '
           'requires 1 <= port && port <= 65535 requires len(request) <= 1000000 '
           'ensures -5 <= result && result < 1000001000000',
    'q02': 'pub fn serve(host: str, ctl: i64, budget: i64) -> i64 ! net, alloc '
           'requires 1 <= ctl && ctl <= 65535 requires 0 <= budget && budget <= 1000 '
           'ensures -3 <= result && result <= 100010001000',
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


# ---- q01 oracle -------------------------------------------------------------------------------------------------
# name -> (python generator, Rust expression producing the same Vec<u8>)
PAYLOADS = {
    'empty': (lambda: b'', 'Vec::<u8>::new()'),
    'a': (lambda: b'a', 'b"a".to_vec()'),
    'hello': (lambda: b'hello', 'b"hello".to_vec()'),
    'all256': (lambda: bytes(range(256)), '(0..=255u8).collect::<Vec<u8>>()'),
    'p100k': (lambda: bytes((i * 7 + 3) % 251 for i in range(100000)),
              '(0..100000usize).map(|i| ((i * 7 + 3) % 251) as u8).collect::<Vec<u8>>()'),
    'p300k': (lambda: bytes((i * 13 + 5) % 256 for i in range(300000)),
              '(0..300000usize).map(|i| ((i * 13 + 5) % 256) as u8).collect::<Vec<u8>>()'),
}


def ref_call(req):
    """Python oracle for q01 against the echo server (payload = every request byte plus 1 mod 256)."""
    reply = bytes((b + 1) % 256 for b in req)
    h = 1
    for b in reply:
        h = (h * 256 + b) % 999983
    return len(reply) * 1000000 + h


# (case name, server mode, payload, expected result)
CALL_CASES = [
    ('echo_empty', 'echo', 'empty', ref_call(PAYLOADS['empty'][0]())),
    ('echo_a', 'echo', 'a', ref_call(PAYLOADS['a'][0]())),
    ('echo_all256', 'echo', 'all256', ref_call(PAYLOADS['all256'][0]())),
    ('echo_100k', 'echo', 'p100k', ref_call(PAYLOADS['p100k'][0]())),
    ('echo_300k', 'echo', 'p300k', ref_call(PAYLOADS['p300k'][0]())),
    ('extra_hello', 'extra', 'hello', ref_call(PAYLOADS['hello'][0]())),
    ('short', 'short', 'hello', -4),
    ('noreply', 'noreply', 'hello', -4),
    ('huge', 'huge', 'hello', -5),
    ('silent', 'silent', 'hello', -3),
]

COMMON = r'''use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

fn check(p: &mut usize, t: &mut usize, name: &str, ok: bool) {
    *t += 1; if ok { *p += 1; } else { println!("FAIL {}", name); }
}
fn secs(n: u64) -> Duration { Duration::from_secs(n) }

// a wrong solution must never hang the grader: the whole driver ends after 90 s
fn watchdog() {
    std::thread::spawn(|| {
        std::thread::sleep(secs(90));
        println!("FAIL watchdog");
        println!("TESTS 0 -1");
        std::process::exit(3);
    });
}

// accept with a bound (never blocks forever)
fn accept_bounded(l: &TcpListener, limit: u64) -> Option<TcpStream> {
    l.set_nonblocking(true).ok()?;
    let end = Instant::now() + secs(limit);
    loop {
        match l.accept() {
            Ok((s, _)) => {
                s.set_nonblocking(false).ok()?;
                s.set_read_timeout(Some(secs(30))).ok()?;
                s.set_write_timeout(Some(secs(30))).ok()?;
                return Some(s);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() > end { return None; }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return None,
        }
    }
}

fn dead_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}
'''

Q01_RS = r'''
// one connection: read the 4-byte header and that many request bytes, answer by mode, return the request
fn serve_one(l: TcpListener, mode: &'static str) -> Option<Vec<u8>> {
    let mut s = accept_bounded(&l, 20)?;
    let _ = s.set_nodelay(true);
    let mut hd = [0u8; 4];
    s.read_exact(&mut hd).ok()?;
    let n = u32::from_be_bytes(hd) as usize;
    let mut req = vec![0u8; n];
    s.read_exact(&mut req).ok()?;
    match mode {
        "echo" | "extra" => {
            let rep: Vec<u8> = req.iter().map(|b| b.wrapping_add(1)).collect();
            let h = (rep.len() as u32).to_be_bytes();
            let _ = s.write_all(&h[..2]);
            let _ = s.flush();
            std::thread::sleep(Duration::from_millis(30));
            let _ = s.write_all(&h[2..]);
            for c in rep.chunks(1021) {
                if s.write_all(c).is_err() { break; }
            }
            if mode == "extra" {
                let _ = s.write_all(&[7u8; 50]);
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        "short" => { let _ = s.write_all(&[0, 0, 0, 10, 1, 2, 3, 4, 5]); }
        "huge" => { let _ = s.write_all(&2000000u32.to_be_bytes()); }
        "silent" => { std::thread::sleep(Duration::from_secs(4)); }
        _ => {}
    }
    Some(req)
}

fn server(mode: &'static str) -> (u16, Receiver<Option<Vec<u8>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = channel();
    std::thread::spawn(move || { let _ = tx.send(serve_one(l, mode)); });
    (port, rx)
}

fn case(p: &mut usize, t: &mut usize, name: &str, mode: &'static str, req: Vec<u8>, want: i64) {
    let (port, rx) = server(mode);
    let r = sol::call("127.0.0.1", port as i64, &req);
    let seen = rx.recv_timeout(secs(30)).ok().flatten();
    let ok = r == want && seen.as_ref() == Some(&req);
    if !ok { eprintln!("{}: got {} want {}, request seen intact: {}", name, r, want, seen.as_ref() == Some(&req)); }
    check(p, t, name, ok);
}

fn main() {
    watchdog();
    let mut passed = 0; let mut total = 0;
'''

Q02_RS = r'''
fn pat(n: usize) -> Vec<u8> {
    (0..n).map(|i| { let b = ((i * 5 + 1) % 251) as u8; if b == 10 { 11 } else { b } }).collect()
}

fn rev_lf(v: &[u8]) -> Vec<u8> {
    let mut r: Vec<u8> = v.iter().rev().cloned().collect();
    r.push(10);
    r
}

// client number idx (1..=10); every client reads the reply to EOF with a 10 s timeout and never sends after its LF
fn client(port: u16, idx: usize) -> Vec<u8> {
    let mut s = None;
    for _ in 0..200 {
        if let Ok(c) = TcpStream::connect(("127.0.0.1", port)) { s = Some(c); break; }
        std::thread::sleep(Duration::from_millis(25));
    }
    let mut s = match s { Some(s) => s, None => return Vec::new() };
    let _ = s.set_read_timeout(Some(secs(10)));
    let _ = s.set_write_timeout(Some(secs(10)));
    let _ = s.set_nodelay(true);
    match idx {
        1 => { let _ = s.write_all(b"hello\n"); }
        2 => {
            let _ = s.write_all(b"ab"); let _ = s.flush();
            std::thread::sleep(Duration::from_millis(50));
            let _ = s.write_all(b"cd\n");
        }
        3 => { let _ = s.write_all(b"\n"); }
        4 => {
            let mut v: Vec<u8> = (0..=255u8).filter(|&b| b != 10).collect();
            v.push(10);
            let _ = s.write_all(&v);
        }
        5 => {
            for c in pat(1024).chunks(7) { let _ = s.write_all(c); }
            let _ = s.write_all(&[10]);
        }
        6 => { let _ = s.write_all(&pat(1025)); }
        7 => { let _ = s.write_all(b"no newline"); let _ = s.shutdown(Shutdown::Write); }
        8 => {}
        9 => { return Vec::new(); }
        _ => { let _ = s.write_all(b"a\r\n"); }
    }
    let mut reply = Vec::new();
    let _ = s.read_to_end(&mut reply);
    reply
}

fn expect(idx: usize) -> Vec<u8> {
    match idx {
        1 => b"olleh\n".to_vec(),
        2 => b"dcba\n".to_vec(),
        3 => b"\n".to_vec(),
        4 => { let v: Vec<u8> = (0..=255u8).filter(|&b| b != 10).collect(); rev_lf(&v) }
        5 => rev_lf(&pat(1024)),
        6 => b"!\n".to_vec(),
        10 => b"\ra\n".to_vec(),
        _ => Vec::new(),
    }
}

// the control listener, the thread that learns the port and runs the first n clients, and the Pruv server on this thread
fn scenario(budget: i64, n: usize) -> (i64, Option<Vec<Vec<u8>>>) {
    let ctl = TcpListener::bind("127.0.0.1:0").unwrap();
    let cport = ctl.local_addr().unwrap().port();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let res = (|| -> Option<Vec<Vec<u8>>> {
            let mut c = accept_bounded(&ctl, 20)?;
            let mut m = [0u8; 2];
            c.read_exact(&mut m).ok()?;
            let port = u16::from_be_bytes(m);
            drop(c);
            let mut out = Vec::new();
            for idx in 1..=n { out.push(client(port, idx)); }
            Some(out)
        })();
        let _ = tx.send(res);
    });
    let r = sol::serve("127.0.0.1", cport as i64, budget);
    let got = rx.recv_timeout(secs(60)).ok().flatten();
    (r, got)
}

fn main() {
    watchdog();
    let mut passed = 0; let mut total = 0;
    let (r, got) = scenario(10, 10);
    let got = got.unwrap_or_default();
    for idx in 1..=10usize {
        if idx == 9 { continue; }
        check(&mut passed, &mut total, &format!("client_{}", idx), got.get(idx - 1) == Some(&expect(idx)));
    }
    check(&mut passed, &mut total, "value", r == 600010003);
    if r != 600010003 { eprintln!("serve returned {}", r); }
    let (r0, got0) = scenario(0, 0);
    check(&mut passed, &mut total, "budget_zero", r0 == 0 && got0 == Some(Vec::new()));
    check(&mut passed, &mut total, "empty_host", sol::serve("", 1, 3) == -1);
    let dead = dead_port();
    check(&mut passed, &mut total, "control_refused", sol::serve("127.0.0.1", dead as i64, 3) == -2);
    println!("TESTS {} {}", passed, total);
}
'''


def driver(pid):
    if pid == 'q01':
        lines = []
        for name, mode, payload, want in CALL_CASES:
            lines.append(f'    case(&mut passed, &mut total, "{name}", "{mode}", {PAYLOADS[payload][1]}, {want});')
        lines += ['    let dead = dead_port();',
                  '    check(&mut passed, &mut total, "refused", sol::call("127.0.0.1", dead as i64, &b"x".to_vec()) == -1);',
                  '    check(&mut passed, &mut total, "empty_host", sol::call("", 80, &Vec::<u8>::new()) == -1);',
                  '    println!("TESTS {} {}", passed, total);', '}']
        # PAYLOADS expressions that are used more than once are fine: every case builds its own Vec
        return COMMON + Q01_RS + '\n'.join(lines) + '\n'
    return COMMON + Q02_RS


def build(c, wd, body):
    extra = []
    if c['ll']:
        try:
            extra, error = G2.build_llvm_objects(c['ll'], 'g10n', str(wd))
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
    with tempfile.TemporaryDirectory(prefix='grade_', dir=WORK) as tmp:
        wd = Path(tmp)
        c = G2.compile_pruv(compiler, str(src), str(wd), backend, flags)
        if c['rc']:
            return dict(c, status='pruv_error')
        exe, error = build(c, wd, driver(pid))
        if error:
            return {'status': 'compile_error', 'error': error}
        r = run([exe], timeout=120)
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
