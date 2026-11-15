// Fixture child for the G10 proc benchmark. Built by run.py and perf.py with rustc -O.
//   --version FLAG   stdout: every argument + LF; stderr: cwd + LF; reads stdin to EOF; exit len(FLAG) % 256
//                    (FLAG "ABORT" aborts after the output, a signal exit on Unix)
//   lines KIND       stdout: the KIND text below, written in small pieces while a thread floods stderr (300 KiB)
//   bulk N           N bytes on each of stdout and stderr in alternating 4096-byte writes, byte i = i % 251
use std::io::{Read, Write};

fn lines(kind: &str) -> Option<Vec<u8>> {
    let mut v: Vec<u8> = Vec::new();
    match kind {
        "empty" => {}
        "one" => v.extend_from_slice(b"abc"),
        "blank" => v.extend_from_slice(b"\n\n\n"),
        "long" => { v.extend(std::iter::repeat(b'x').take(20000)); v.extend_from_slice(b"\nyz"); }
        "mix" => {
            for i in 0..300usize {
                v.extend(std::iter::repeat(b'a' + (i % 26) as u8).take((i * 37) % 5000));
                if i != 299 { v.push(b'\n'); }
            }
        }
        "bin" => { v.extend((0u8..=255).filter(|b| *b != 10)); v.extend_from_slice(b"\n\r\n\r"); }
        "big" => { for _ in 0..3000 { v.extend(std::iter::repeat(b'q').take(999)); v.push(b'\n'); } }
        "fail" => v.extend_from_slice(b"partial\n"),
        "forever" => {}
        _ => return None,
    }
    Some(v)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = std::io::stdout().lock();
    match args.first().map(|s| s.as_str()) {
        Some("--version") => {
            for a in &args { out.write_all(a.as_bytes()).unwrap(); out.write_all(b"\n").unwrap(); }
            out.flush().unwrap();
            let cwd = std::env::current_dir().unwrap();
            eprintln!("{}", cwd.to_string_lossy());
            let mut sink = Vec::new();
            std::io::stdin().read_to_end(&mut sink).unwrap();
            let flag = args.get(1).cloned().unwrap_or_default();
            if flag == "ABORT" { std::process::abort(); }
            std::process::exit((flag.len() % 256) as i32);
        }
        Some("lines") => {
            let kind = args.get(1).cloned().unwrap_or_default();
            let text = match lines(&kind) { Some(t) => t, None => std::process::exit(2) };
            let flood = std::thread::spawn(|| {
                let mut e = std::io::stderr().lock();
                let block = vec![b'e'; 1024];
                for _ in 0..300 { if e.write_all(&block).is_err() { break; } }
            });
            if kind == "forever" {
                loop { if out.write_all(b"y\n").is_err() { std::process::exit(0); } }
            }
            for piece in text.chunks(1777) { out.write_all(piece).unwrap(); out.flush().unwrap(); }
            drop(out);
            flood.join().unwrap();
            std::process::exit(if kind == "fail" { 3 } else { 0 });
        }
        Some("bulk") => {
            let n: usize = args[1].parse().unwrap();
            let data: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
            let mut err = std::io::stderr().lock();
            for c in data.chunks(4096) { out.write_all(c).unwrap(); err.write_all(c).unwrap(); }
            out.flush().unwrap();
        }
        _ => std::process::exit(2),
    }
}
