"""L3-2 A3: the whole supervisor with real Python children and a Rust host playing the controller.

Each line: mode delay_ms shell argv cwd out file_limit tail_chars (strings hex, '-' = empty).
mode: wait (hold the control socket until the call returns), stop (send one byte after delay), close (close
after delay), noconn (nobody listens), zero (control port 0).
Output: packet | started byte seen | output file summary.
"""
import os
import sys

from common import HERE, manifest_flags, run_lines
from checker import parse_packet

HARNESS = r'''
#[path="@SOL@"] mod sol;
use std::io::{BufRead, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn unhex(w: &str) -> String {
 if w == "-" { return String::new(); }
 let b: Vec<u8> = (0..w.len() / 2).map(|i| u8::from_str_radix(&w[2 * i..2 * i + 2], 16).unwrap()).collect();
 String::from_utf8(b).unwrap()
}

fn summary(path: &str) -> String {
 match std::fs::read(path) {
  Err(_) => "-".to_string(),
  Ok(b) if b.len() <= 64 => format!("={}", b.iter().map(|x| format!("{:02x}", x)).collect::<String>()),
  Ok(b) => {
   let mut h: u64 = 0xcbf29ce484222325;
   for x in &b { h ^= *x as u64; h = h.wrapping_mul(0x100000001b3); }
   format!("{}:{:016x}", b.len(), h)
  }
 }
}

fn main() {
 let stdin = std::io::stdin();
 for line in stdin.lock().lines() {
  let line = line.unwrap();
  let w: Vec<&str> = line.split(' ').collect();
  let mode = w[0];
  let delay = Duration::from_millis(w[1].parse().unwrap());
  let (shell, argv, cwd, out) = (unhex(w[2]), unhex(w[3]), unhex(w[4]), unhex(w[5]));
  let limit: i64 = w[6].parse().unwrap();
  let tail: i64 = w[7].parse().unwrap();
  let done = Arc::new(AtomicBool::new(false));
  let mut port: i64 = 0;
  let mut host = None;
  if mode == "noconn" {
   let l = TcpListener::bind("127.0.0.1:0").unwrap();
   port = l.local_addr().unwrap().port() as i64;
   drop(l);
  } else if mode != "zero" {
   let l = TcpListener::bind("127.0.0.1:0").unwrap();
   port = l.local_addr().unwrap().port() as i64;
   l.set_nonblocking(true).unwrap();
   let done2 = done.clone();
   let mode2 = mode.to_string();
   host = Some(std::thread::spawn(move || -> bool {
    let mut s = loop {
     match l.accept() {
      Ok((s, _)) => break s,
      Err(_) => {
       if done2.load(Ordering::SeqCst) { return false; }
       std::thread::sleep(Duration::from_millis(2));
      }
     }
    };
    s.set_nonblocking(false).unwrap();
    let mut b = [0u8; 1];
    let started = matches!(s.read(&mut b), Ok(1)) && b[0] == 1;
    if started && mode2 != "wait" {
     std::thread::sleep(delay);
     if mode2 == "stop" { let _ = s.write_all(&[1]); } else { drop(s); return started; }
    }
    while !done2.load(Ordering::SeqCst) { std::thread::sleep(Duration::from_millis(2)); }
    started
   }));
  }
  let _ = std::fs::remove_file(&out);
  let packet = sol::supervise(&shell, &argv, &cwd, &out, limit, tail, port);
  done.store(true, Ordering::SeqCst);
  let started = host.map(|h| h.join().unwrap()).unwrap_or(false);
  let text: Vec<String> = packet.iter().map(|x| x.to_string()).collect();
  println!("{} | {} | {}", text.join(" "), started as i64, summary(&out));
  std::io::stdout().flush().unwrap();
 }
}
'''

WORK = HERE / 'work' / 'a3'
PY = sys.executable


def hx(s):
    b = s.encode() if isinstance(s, str) else s
    return b.hex() if b else '-'


def summary(data):
    if data is None:
        return '-'
    if len(data) <= 64:
        return '=' + data.hex()
    h = 0xcbf29ce484222325
    for x in data:
        h = ((h ^ x) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return f'{len(data)}:{h:016x}'


def tail_of(text, t):
    units = text.encode('utf-16-le', 'surrogatepass')
    cut = units[-2 * t:] if len(units) > 2 * t else units
    if len(cut) >= 2 and 0xDC00 <= int.from_bytes(cut[:2], 'little') <= 0xDFFF:
        cut = cut[2:]
    return cut.decode('utf-16-le').encode()


def packet(kind, exit_status=0, known=0, stopped=0, reaped=0, truncated=0, file_error=0, file_open=0, total=0,
           file_bytes=0, tail=b'', status=0):
    h = [status, kind, exit_status, known, stopped, reaped, truncated, file_error, file_open, total, file_bytes]
    return h + [len(tail)] + list(tail)


def blob(args):
    return ''.join(a + '\0' for a in args)


def main():
    (WORK / 'cwd').mkdir(parents=True, exist_ok=True)
    cases = []

    def add(label, mode, delay, shell, args, expect, started, file_data, cwd='', limit=1 << 20, tail=64,
            out=None):
        out = out or str(WORK / f'out{len(cases)}.bin')
        line = ' '.join([mode, str(delay), hx(shell), hx(blob(args) if isinstance(args, list) else args), hx(cwd),
                         hx(out), str(limit), str(tail)])
        p = expect
        if p[0] == 0:
            parse_packet(p, tail)
        cases.append((label, line, f'{" ".join(map(str, p))} | {started} | {summary(file_data)}'))

    def run(label, code, out_bytes, exit_status=0, limit=1 << 20, tail=64, text=None, cwd='', extra=(),
            create_ok=True, out=None):
        """A child that runs to its end; out_bytes is everything it writes, in read order."""
        written = out_bytes[:limit]
        text = out_bytes.decode('utf-8', 'replace') if text is None else text
        p = packet(1, exit_status, 1, 0, 1, int(len(out_bytes) > limit), 0 if create_ok else 1, int(create_ok),
                   len(out_bytes), len(written) if create_ok else 0, tail_of(text, tail))
        add(label, 'wait', 0, PY, ['-c', code, *extra], p, 1, written if create_ok else None, cwd, limit, tail,
            out)

    run('hello', 'print("hello")', b'hello\n')
    run('empty', 'pass', b'')
    run('exit 7', 'import sys; sys.stdout.write("x"); sys.exit(7)', b'x', 7)
    run('stderr', 'import sys; sys.stderr.write("err"); sys.exit(3)', b'err', 3)
    run('signal', 'import os,signal,sys; sys.stdout.write("a"); sys.stdout.flush(); os.kill(os.getpid(), 9)',
        b'a', -9)
    run('two streams', 'import sys,time; sys.stdout.write("A"*10); sys.stdout.flush(); time.sleep(0.3); '
        'sys.stderr.write("B"*10)', b'A' * 10 + b'B' * 10)
    big = bytes((i * 7 + i // 251) % 256 for i in range(3000000))
    run('big truncated', 'import sys; sys.stdout.buffer.write(bytes((i*7+i//251)%256 for i in range(3000000)))',
        big, limit=1000000, tail=500)
    run('big fits', 'import sys; sys.stdout.buffer.write(b"0123456789"*200000)', b'0123456789' * 200000,
        limit=2000000, tail=7)
    run('limit 0', 'import sys; sys.stdout.write("abc")', b'abc', limit=0)
    run('exact fit', 'import sys; sys.stdout.write("hello")', b'hello', limit=5)
    run('over by one', 'import sys; sys.stdout.write("hello")', b'hello', limit=4)
    run('invalid utf8', r'import sys; sys.stdout.buffer.write(b"\xe2\x82 ok \xf0\x9f\x98\x80 \xff\xc3")',
        b'\xe2\x82 ok \xf0\x9f\x98\x80 \xff\xc3', tail=6)
    run('surrogate cut', r'import sys; sys.stdout.buffer.write("\U0001F600".encode()*3)',
        '\U0001F600'.encode() * 3, tail=3)
    run('tail 1', 'import sys; sys.stdout.write("abc한")', 'abc한'.encode(), tail=1)
    run('tail max', 'import sys; sys.stdout.write("z"*70000)', b'z' * 70000, tail=65536)
    # one decoder per stream: a sequence split on stdout around a stderr byte still decodes
    run('split across streams', r'import sys,time; o=sys.stdout.buffer; o.write(b"\xe2\x82"); o.flush(); '
        r'time.sleep(0.3); sys.stderr.write("x"); sys.stderr.flush(); time.sleep(0.3); o.write(b"\xac")',
        b'\xe2\x82x\xac', text='x€')
    # stdin is closed at once, so a child that reads it ends
    run('stdin eof', 'import sys; print(len(sys.stdin.read()))', b'0\n')
    cwd = str(WORK / 'cwd')
    run('cwd', 'import os; print(os.getcwd())', (cwd + '\n').encode(), cwd=cwd)
    run('args', 'import sys; print(sys.argv[1:])', (repr(['', 'a b', '한글 😀', '-x']) + '\n').encode(),
        extra=['', 'a b', '한글 😀', '-x'])
    run('create fail', 'print("still runs")', b'still runs\n', create_ok=False,
        out=str(WORK / 'missing' / 'x.bin'))

    # spawn failures: the output file is created and stays empty; no started byte
    add('spawn missing', 'wait', 0, '/nonexistent/prog', [], packet(2, 2, file_open=1), 0, b'')
    add('spawn empty', 'wait', 0, '', [], packet(2, 1, file_open=1), 0, b'')
    add('spawn create fail', 'wait', 0, '/nonexistent/prog', [], packet(2, 2, file_error=1), 0, None,
        out=str(WORK / 'missing' / 'y.bin'))

    # stop and disconnect: queued output is taken, the shell is killed and reaped, no exit status
    hold = 'import sys,time; sys.stdout.write("ready\\n"); sys.stdout.flush(); time.sleep(30)'
    add('stop', 'stop', 500, PY, ['-c', hold], packet(1, stopped=1, reaped=1, file_open=1, total=6, file_bytes=6,
                                                       tail=b'ready\n'), 1, b'ready\n')
    add('disconnect', 'close', 500, PY, ['-c', hold],
        packet(1, stopped=2, reaped=1, file_open=1, total=6, file_bytes=6, tail=b'ready\n'), 1, b'ready\n')
    quiet = 'import time; time.sleep(30)'
    add('early close', 'close', 0, PY, ['-c', quiet], packet(1, stopped=2, reaped=1, file_open=1), 1, b'')
    add('stop quiet', 'stop', 200, PY, ['-c', quiet], packet(1, stopped=1, reaped=1, file_open=1), 1, b'')
    add('stop truncated', 'stop', 500, PY, ['-c', hold],
        packet(1, stopped=1, reaped=1, truncated=1, file_open=1, total=6, file_bytes=2, tail=b'y\n'), 1, b're',
        limit=2, tail=2)

    # control connect failure and bad input: the output file is not touched
    add('no controller', 'noconn', 0, PY, ['-c', 'pass'], packet(4), 0, None)
    bad = packet(5, status=1)
    add('tail 0', 'wait', 0, PY, ['-c', 'pass'], bad, 0, None, tail=0)
    add('tail too big', 'wait', 0, PY, ['-c', 'pass'], bad, 0, None, tail=65537)
    add('limit negative', 'wait', 0, PY, ['-c', 'pass'], bad, 0, None, limit=-1)
    add('limit too big', 'wait', 0, PY, ['-c', 'pass'], bad, 0, None, limit=(1 << 40) + 1)
    add('argv no nul', 'wait', 0, PY, '-c', bad, 0, None)
    add('control port 0', 'zero', 0, PY, ['-c', 'pass'], bad, 0, None)
    for p in WORK.glob('out*.bin'):
        p.unlink()
    return run_lines('a3', HERE / 'supervise.pruv', HARNESS, cases, flags=manifest_flags(HERE / 'supervise.pruv'))


if __name__ == '__main__':
    sys.exit(main())
