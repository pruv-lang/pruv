"""G10 async benchmark; generated files stay under bench/g10_async/.work.

PRUV_PYTHON, PRUVC_RS, RUSTC and CLANG can select existing tools without building/installing.
The Rust host drives loopback peers; q02 also starts a local fixture child.
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
                         if (ROOT / '.venv/bin/python').exists() else '/Users/spica4345/Projects/pruv/.venv/bin/python'), str(ROOT / 'pruvc/pruvc.py')]
G2.COMPILERS['rs'] = [os.environ.get('PRUVC_RS', str(ROOT / 'pruvc-rs/target/release/pruvc'))]
G2.RUSTC = os.environ.get('RUSTC', str(Path.home() / '.cargo/bin/rustc'))
G2.CLANG = os.environ.get('CLANG', G2.CLANG)
PROBLEMS = ['q01', 'q02']

def run(cmd, timeout=180, env=None, cwd=None):
    try:
        r = subprocess.run([str(x) for x in cmd], capture_output=True, text=True,
                           timeout=timeout, env=env, cwd=cwd, errors='replace')
        return {'rc': r.returncode, 'stdout': r.stdout, 'stderr': r.stderr}
    except subprocess.TimeoutExpired:
        return {'rc': -1, 'stdout': '', 'stderr': 'timeout'}
    except OSError as e:
        return {'rc': -2, 'stdout': '', 'stderr': str(e)}
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


DRIVER = r"""
fn connect(port: u16) -> TcpStream {
    let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(secs(15))).unwrap();
    s.set_write_timeout(Some(secs(5))).unwrap();
    s
}
fn announce(l: &TcpListener) -> u16 {
    let mut c = accept_bounded(l, 10).unwrap();
    let mut b = [0u8;2]; c.read_exact(&mut b).unwrap(); u16::from_be_bytes(b)
}
"""
Q01 = r"""
fn main() {
    watchdog();
    let ctl = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = ctl.local_addr().unwrap().port();
    let clients = std::thread::spawn(move || {
        let port = announce(&ctl);
        let mut silent = connect(port);
        let active: Vec<_> = (0..7).map(|_| connect(port)).collect();
        let workers: Vec<_> = active.into_iter().enumerate().map(|(i,mut s)| std::thread::spawn(move || {
            let mut pass = 0;
            let _ = s.write_all(&[0,255,13]);
            std::thread::sleep(Duration::from_millis(15));
            let _ = s.write_all(b"\nabc\n\n");
            let mut out = [0u8;6];
            if s.read_exact(&mut out).is_ok() && out == [0,3,0,3,0,0] { pass += 1; }
            // EOF after queued bytes must not discard those bytes.
            if i == 0 { let _ = s.write_all(b"tail\n"); let _ = s.shutdown(Shutdown::Write);
                let mut last=[0;2]; if s.read_exact(&mut last).is_ok() && last == [0,4] {pass+=1;} }
            if i == 1 {
                for _ in 0..3 { let _ = s.write_all(b"x"); std::thread::sleep(Duration::from_millis(180)); }
                let _ = s.write_all(b"\n"); let mut b=[0;2];
                if s.read_exact(&mut b).is_ok() && b == [0,3] {pass+=1;}
            }
            if i == 2 {
                let _ = s.write_all(&vec![255;1024]); let _ = s.write_all(b"\n");
                let mut b=[0;2]; if s.read_exact(&mut b).is_ok() && b == [4,0] {pass+=1;}
            }
            pass
        })).collect();
        let mut pass: usize = workers.into_iter().map(|w| w.join().unwrap_or(0)).sum();
        // The silent client must time out while other clients still exist.
        let mut b=[0u8;1]; if silent.read(&mut b).ok() == Some(0) {pass+=1;}
        pass
    });
    let value = sol::serve(port as i64, 8);
    let n = clients.join().unwrap();
    let mut p=0; let mut t=0;
    check(&mut p,&mut t,"clients_chunks_lf_eof_timeout_rearm_boundary",n==11);
    check(&mut p,&mut t,"budget",value==8);
    let ctl0=TcpListener::bind("127.0.0.1:0").unwrap();
    let port0=ctl0.local_addr().unwrap().port();
    let zero=std::thread::spawn(move || announce(&ctl0));
    check(&mut p,&mut t,"budget_zero",sol::serve(port0 as i64,0)==0 && zero.join().unwrap()>0);
    println!("TESTS {} {}",p,t);
}
"""
Q02 = r"""
fn scenario(program: &str, deadline: i64, cancel: bool) -> (i64, Vec<u8>) {
    let ctl = TcpListener::bind("127.0.0.1:0").unwrap();
    let port=ctl.local_addr().unwrap().port();
    let client=std::thread::spawn(move || {
        let mut s=connect(announce(&ctl));
        let mut out=Vec::new();
        if cancel {
            let mut prefix=[0u8;8];
            if s.read_exact(&mut prefix).is_ok() { out.extend_from_slice(&prefix); }
            let _ = s.write_all(b"C");
        }
        let _ = s.read_to_end(&mut out); out
    });
    let value=sol::supervise(program,port as i64,deadline);
    (value,client.join().unwrap())
}
fn main() {
    watchdog(); let mut p=0; let mut t=0;
    for (program,deadline,cancel,tag,status) in [
        (@EXIT@,5000,false,1u8,7i64),
        (@SLOW@,5000,true,4u8,0i64),
        (@SLOW@,600,false,2u8,0i64)] {
        let (r,out)=scenario(program,deadline,cancel);
        let mut want=b"one\ntwo\n".to_vec(); want.push(tag);
        want.extend_from_slice(&((status+2147483648) as u64).to_be_bytes());
        check(&mut p,&mut t,&format!("supervisor_{}",tag),r==tag as i64 && out==want);
    }
    println!("TESTS {} {}",p,t);
}
"""

def driver(pid):
    if pid == 'q01':
        return COMMON + DRIVER + Q01
    return COMMON + DRIVER + Q02.replace('@EXIT@', json.dumps(str(HERE / 'fixtures/exit.py'))).replace('@SLOW@', json.dumps(str(HERE / 'fixtures/slow.py')))

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


def grade_one(src, pid, compiler, backend, flags=()):
    src = Path(src).resolve()
    if not src.exists():
        return {'status': 'missing'}
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
        return dict(r, status='bind_permission' if 'Operation not permitted' in r['stderr'] or 'Permission denied' in r['stderr'] else ('ok' if r['rc'] == 0 and passed == total and total > 0 else 'wrong'),
                    tests_passed=passed, tests_total=total)



def main():
    p = argparse.ArgumentParser(__doc__)
    p.add_argument('solutions', type=Path, nargs='?', default=HERE / 'solutions')
    p.add_argument('out', type=Path, nargs='?')
    p.add_argument('--compiler', choices=['py', 'rs', 'both'], default='both')
    p.add_argument('--backend', choices=['rust', 'llvm', 'both'], default='both')
    a = p.parse_args()
    failed = False
    for compiler in ['py','rs'] if a.compiler == 'both' else [a.compiler]:
        for backend in ['rust','llvm'] if a.backend == 'both' else [a.backend]:
            rows = {}
            for pid in PROBLEMS:
                r = grade_one(a.solutions / (pid + '.pruv'), pid, compiler, backend)
                rows[pid] = r
                print(compiler+'-'+backend, pid, r['status'], str(r.get('tests_passed','-'))+'/'+str(r.get('tests_total','-')), flush=True)
                if r['status'] != 'ok':
                    print(r.get('stderr') or r.get('errors') or r.get('error'))
                    failed = True
            out = a.out or HERE / 'results' / ('solutions_'+compiler+'_'+backend+'.json')
            out.parent.mkdir(exist_ok=True)
            out.write_text(json.dumps({'tools':G2.COMPILERS,'results':{compiler+'-'+backend:rows}},indent=2))
    return int(failed)

if __name__ == '__main__':
    sys.exit(main())
