// G10 proc A2 fixture child (std only). tests/run_all.py compiles it once with rustc into the work directory and passes
// its absolute path to the g10p runtime drivers in the environment variable PRUV_PROC_CHILD.
//
// argv[1] selects the mode; the other arguments belong to the mode:
//   args A...         print every argument after the mode, each followed by one LF, with its exact bytes
//   cwd               print the current directory followed by LF
//   echo              copy stdin to stdout until EOF, exit 0
//   count             read stdin until EOF, print its length in decimal and LF
//   errout N M        write N pattern bytes to stderr, then M pattern bytes to stdout, exit 0
//   flood N M         write N pattern bytes to both stdout and stderr (alternating 4096-byte blocks) BEFORE reading
//                     stdin, then read stdin to EOF; exit 0 when it is exactly M pattern(0) bytes, 17 otherwise
//   exit CODE         exit with CODE at once
//   quick             exit 0 at once without reading stdin (a later stdin write hits a closed pipe)
//   hold              print "ready" and LF on stdout, then sleep until killed (gives up after two minutes with exit 9)
//   sleep             like hold without the output
//   close_out         close stdout, sleep a little, exit 3 (stdout EOF arrives while the child still runs)
//   touch MS FILE     sleep MS milliseconds, then create FILE, exit 0 (a killed child never creates it)
//   take N [FILE]     read exactly N bytes of stdin (exit 18 on an early EOF), write them to FILE when given, print them to
//                     stdout, then exit 0 WITHOUT reading more (a larger stdin write fails half way through with a broken
//                     pipe, and FILE shows what arrived; a failed Pruv handle publishes no output, hence the file)
//   pidfile FILE      write std::process::id() to FILE (via FILE.tmp and rename, so FILE is never half written), then read
//                     stdin to EOF, exit 0 (a driver can tell whether the child ran, and whether it was reaped)
//   exit_mark CODE FILE  write its pid to FILE, then exit CODE at once (FILE is the very last observable action before the exit)
//   waitfile FILE     poll up to 10 s for a pid in FILE, then until `ps` shows that process as a zombie (it has exited), and exit
//                     0 (exit 19 on a timeout). Used as a
//                     synchronisation helper inside a Pruv function, which has no sleep of its own
//
// pattern(stream, i) = (i * 31 + stream * 7 + 5) % 251; the drivers and the Pruv tests use the same formula.
use std::io::{Read, Write};

#[cfg(unix)]
fn exited(pid: &str) -> bool {
    match std::process::Command::new("ps").args(["-o", "stat=", "-p", pid]).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim_start().starts_with('Z'),
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn exited(_pid: &str) -> bool {
    sleep_ms(200);
    true
}

fn pat(stream: u64, i: u64) -> u8 {
    ((i.wrapping_mul(31).wrapping_add(stream * 7 + 5)) % 251) as u8
}

fn block(stream: u64, start: u64, n: usize) -> Vec<u8> {
    (0..n).map(|j| pat(stream, start + j as u64)).collect()
}

fn put(stream: u64, data: &[u8]) {
    if stream == 1 {
        let so = std::io::stdout();
        let mut l = so.lock();
        l.write_all(data).unwrap();
        l.flush().unwrap();
    } else {
        let se = std::io::stderr();
        let mut l = se.lock();
        l.write_all(data).unwrap();
        l.flush().unwrap();
    }
}

fn emit(stream: u64, n: u64) {
    let mut i: u64 = 0;
    while i < n {
        let k = std::cmp::min(4096, n - i) as usize;
        put(stream, &block(stream, i, k));
        i += k as u64;
    }
}

#[cfg(unix)]
fn os_bytes(a: &std::ffi::OsStr) -> Vec<u8> {
    std::os::unix::ffi::OsStrExt::as_bytes(a).to_vec()
}

#[cfg(not(unix))]
fn os_bytes(a: &std::ffi::OsStr) -> Vec<u8> {
    a.to_string_lossy().as_bytes().to_vec()
}

#[cfg(unix)]
fn close_stdout() {
    extern "C" {
        fn close(fd: i32) -> i32;
    }
    unsafe {
        close(1);
    }
}

#[cfg(not(unix))]
fn close_stdout() {}

fn sleep_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let mode = args.get(1).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let num = |k: usize| -> u64 { args.get(k).and_then(|s| s.to_str()).and_then(|s| s.parse().ok()).unwrap_or(0) };
    match mode.as_str() {
        "async_flood" => {
            for i in 0..512u64 {
                put(1, &block(1, i*4096, 4096));
                put(2, &block(2, i*4096, 4096));
            }
        }
        "args" => {
            let mut out: Vec<u8> = Vec::new();
            for a in args.iter().skip(2) {
                out.extend_from_slice(&os_bytes(a));
                out.push(b'\n');
            }
            put(1, &out);
        }
        "cwd" => {
            let mut out = os_bytes(std::env::current_dir().unwrap().as_os_str());
            out.push(b'\n');
            put(1, &out);
        }
        "echo" => {
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf).unwrap();
            put(1, &buf);
        }
        "count" => {
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf).unwrap();
            put(1, format!("{}\n", buf.len()).as_bytes());
        }
        "errout" => {
            emit(2, num(2));
            emit(1, num(3));
        }
        "flood" => {
            let n = num(2);
            let mut i: u64 = 0;
            while i < n {
                let k = std::cmp::min(4096, n - i) as usize;
                put(1, &block(1, i, k));
                put(2, &block(2, i, k));
                i += k as u64;
            }
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf).unwrap();
            let good = buf.len() as u64 == num(3) && buf.iter().enumerate().all(|(i, b)| *b == pat(0, i as u64));
            std::process::exit(if good { 0 } else { 17 });
        }
        "exit" => {
            std::process::exit(num(2) as i32);
        }
        "quick" => {}
        "hold" => {
            put(1, b"ready\n");
            for _ in 0..1200 {
                sleep_ms(100);
            }
            std::process::exit(9);
        }
        "sleep" => {
            for _ in 0..1200 {
                sleep_ms(100);
            }
            std::process::exit(9);
        }
        "close_out" => {
            close_stdout();
            sleep_ms(400);
            std::process::exit(3);
        }
        "touch" => {
            sleep_ms(num(2));
            let path = args.get(3).cloned().unwrap_or_default();
            std::fs::write(path, b"x").unwrap();
        }
        "take" => {
            let n = num(2) as usize;
            let mut buf = vec![0u8; n];
            if std::io::stdin().read_exact(&mut buf).is_err() {
                std::process::exit(18);
            }
            if let Some(path) = args.get(3) {
                std::fs::write(path, &buf).unwrap();
            }
            put(1, &buf);
        }
        "pidfile" => {
            let path = args.get(2).cloned().unwrap_or_default();
            let mut tmp = path.clone();
            tmp.push(".tmp");
            std::fs::write(&tmp, std::process::id().to_string()).unwrap();
            std::fs::rename(&tmp, &path).unwrap();
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf).unwrap();
        }
        "exit_mark" => {
            let path = args.get(3).cloned().unwrap_or_default();
            std::fs::write(path, std::process::id().to_string()).unwrap();
            std::process::exit(num(2) as i32);
        }
        "waitfile" => {
            let path = args.get(2).cloned().unwrap_or_default();
            // wait for the pid in FILE, then until that process is a zombie (it has exited; its parent has not reaped it)
            for _ in 0..1000 {
                let pid = std::fs::read_to_string(&path).unwrap_or_default();
                if !pid.trim().is_empty() {
                    if exited(pid.trim()) {
                        return;
                    }
                }
                sleep_ms(10);
            }
            std::process::exit(19);
        }
        _ => std::process::exit(64),
    }
}
