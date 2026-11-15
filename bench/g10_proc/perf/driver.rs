use std::hint::black_box;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Instant;

// Hand-written Rust doing the same work: spawn `child bulk N`, drain stderr on a thread and stdout here, 4096-byte
// reads, mode 0 checks every byte, mode 1 copies each chunk and checks its first and last byte.
fn drain(mut r: impl Read, mode: i64) -> i64 {
    let mut buf = [0u8; 4096];
    let mut off: usize = 0;
    loop {
        let k = match r.read(&mut buf) { Ok(k) => k, Err(_) => return -3 };
        if k == 0 { return off as i64; }
        if mode == 0 {
            for (i, b) in buf[..k].iter().enumerate() { if *b as usize != (off + i) % 251 { return -5; } }
        } else {
            let c: Vec<u8> = buf[..k].to_vec();
            if c[0] as usize != off % 251 || c[k - 1] as usize != (off + k - 1) % 251 { return -5; }
            black_box(&c);
        }
        off += k;
    }
}

fn native(program: &str, n: &str, mode: i64) -> i64 {
    let mut c = match Command::new(program).args(["bulk", n]).stdin(Stdio::piped()).stdout(Stdio::piped())
        .stderr(Stdio::piped()).spawn() { Ok(c) => c, Err(_) => return -1 };
    drop(c.stdin.take());
    let err = c.stderr.take().unwrap();
    let t = std::thread::spawn(move || drain(err, mode));
    let out = drain(c.stdout.take().unwrap(), mode);
    let err = t.join().unwrap();
    let st = c.wait();
    if out < 0 { return out; }
    if err < 0 { return err - 1; }
    match st { Ok(s) if s.code() == Some(0) => out + err, Ok(_) => -7, Err(_) => -8 }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let child = std::env::var("PRUV_PROC_CHILD").unwrap();
    let n = &args[1];
    let mode: i64 = args[2].parse().unwrap();
    let repeat: usize = args[3].parse().unwrap();
    let side = args.get(4).map(|s| s.as_str()).unwrap_or("both");
    let expected = 2 * n.parse::<i64>().unwrap();
    if side != "both" {
        // one run of one side, for a memory measurement of this process tree
        let got = if side == "pruv" { sol::pump(&child, n, mode) } else { native(&child, n, mode) };
        assert_eq!(got, expected);
        println!("{{\"side\":\"{}\",\"total\":{}}}", side, got);
        return;
    }
    assert_eq!(native(&child, n, mode), expected);
    assert_eq!(sol::pump(&child, n, mode), expected);
    let mut pruv = Vec::new();
    let mut rust = Vec::new();
    for round in 0..repeat {
        for s in if round % 2 == 0 { [0, 1] } else { [1, 0] } {
            let start = Instant::now();
            let got = if s == 0 { sol::pump(black_box(&child), n, mode) } else { native(black_box(&child), n, mode) };
            let elapsed = start.elapsed().as_nanos();
            assert_eq!(black_box(got), expected);
            if s == 0 { pruv.push(elapsed); } else { rust.push(elapsed); }
        }
    }
    println!("{{\"total\":{},\"pruv_ns\":{:?},\"rust_ns\":{:?}}}", expected, pruv, rust);
}
