use std::hint::black_box;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const N: usize = 1048576;
const MOD: i64 = 1000000007;

fn pattern() -> Vec<u8> {
    (0..N).map(|off| (off % 251) as u8).collect()
}

// Server: accepts one connection on a fresh 127.0.0.1 port and writes the pattern in 65536-byte write_all calls, then closes.
fn server(data: &'static [u8]) -> (u16, JoinHandle<()>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let h = std::thread::spawn(move || {
        let (mut s, _) = match l.accept() { Ok(x) => x, Err(_) => return };
        let t = Some(Duration::from_secs(30));
        let _ = s.set_write_timeout(t);
        for chunk in data.chunks(65536) {
            if s.write_all(chunk).is_err() { return; }
        }
    });
    (port, h)
}

// Hand-written Rust client: 4096-byte reads until 0; mode 0 folds every byte into the checksum, mode 1 copies each chunk
// and uses its first byte, last byte and length. Same result encoding as the Pruv pump.
fn native(port: u16, mode: i64) -> i64 {
    let mut s = match TcpStream::connect(("127.0.0.1", port)) { Ok(s) => s, Err(_) => return -1 };
    let t = Some(Duration::from_secs(30));
    if s.set_read_timeout(t).is_err() || s.set_write_timeout(t).is_err() { return -2; }
    let mut buf = [0u8; 4096];
    let mut total: i64 = 0;
    let mut sum: i64 = 0;
    let mut acc: i64 = 0;
    let mut first: i64 = 0;
    loop {
        let k = match s.read(&mut buf) { Ok(k) => k, Err(_) => return -2 };
        if k == 0 { break; }
        if total == 0 { first = buf[0] as i64; }
        if mode == 0 {
            for b in &buf[..k] { sum = (sum * 31 + *b as i64) % MOD; }
        } else {
            let c: Vec<u8> = buf[..k].to_vec();
            acc = (acc + c[0] as i64 + c[k - 1] as i64 + k as i64) % MOD;
            black_box(&c);
        }
        total += k as i64;
    }
    black_box(acc);
    if mode == 0 { sum } else { total * 1000 + first }
}

fn run_side(data: &'static [u8], side: &str, mode: i64) -> i64 {
    let (port, h) = server(data);
    let got = if side == "pruv" { sol::pump("127.0.0.1", port as i64, mode) } else { native(port, mode) };
    if got >= 0 { h.join().unwrap(); }
    got
}

fn median(v: &mut Vec<u128>) -> u128 {
    v.sort();
    v[v.len() / 2]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode: i64 = args[1].parse().unwrap();
    let rounds: usize = args[2].parse().unwrap();
    let side = args.get(3).map(|s| s.as_str()).unwrap_or("both");
    let data: &'static [u8] = Box::leak(pattern().into_boxed_slice());
    // the exact value both clients must return
    let expected: i64 = if mode == 0 {
        data.iter().fold(0i64, |s, b| (s * 31 + *b as i64) % MOD)
    } else {
        (N as i64) * 1000 + data[0] as i64
    };
    if side != "both" {
        // one run of one side, for a memory measurement of this process
        let got = run_side(data, side, mode);
        assert_eq!(got, expected, "{} result", side);
        println!("{{\"side\":\"{}\",\"total\":{}}}", side, N);
        return;
    }
    assert_eq!(run_side(data, "rust", mode), expected, "rust warmup");
    assert_eq!(run_side(data, "pruv", mode), expected, "pruv warmup");
    let mut pruv: Vec<u128> = Vec::new();
    let mut rust: Vec<u128> = Vec::new();
    for round in 0..rounds {
        for s in if round % 2 == 0 { [0, 1] } else { [1, 0] } {
            // the server is set up before the clock starts; the timed part is connect, read to EOF and close
            let (port, h) = server(data);
            let start = Instant::now();
            let got = if s == 0 { sol::pump(black_box("127.0.0.1"), port as i64, mode) } else { native(black_box(port), mode) };
            let elapsed = start.elapsed().as_nanos();
            assert_eq!(black_box(got), expected, "{} result", if s == 0 { "pruv" } else { "rust" });
            h.join().unwrap();
            if s == 0 { pruv.push(elapsed); } else { rust.push(elapsed); }
        }
    }
    println!("{{\"total\":{},\"pruv_ns\":{},\"rust_ns\":{}}}", N, median(&mut pruv), median(&mut rust));
}
