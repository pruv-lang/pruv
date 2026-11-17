// M3 driver (appended to a main file that already declares `mod sol;`, see run.py).
//
// stdin mode (no arguments): one case per line, `<kind> <w0> <w1> ...` with kind call|resume|outcome.
// For each line it calls the matching entry with an out buffer of len(inp)+16 words and prints
// `<status> <out words 0..status>` (negative status: the status alone).
//
// `perf <reps>` mode: times sol::m3_call on a typical 3-argument call and prints one `m3_call_ns=..` line.
// The host only moves words; every decision is made by the Pruv entries.
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::time::Instant;

fn typical_call() -> Vec<i64> {
    // [MAGIC_CALL, VERSION, total_len, op=charge(6), now_ms, uuid_len=36, uuid..., argc=3, V...]
    let mut v: Vec<i64> = vec![0x42554443, 1, 0, 6, 1_700_000_000_000, 36];
    for i in 0..36i64 {
        v.push(97 + (i % 26));
    }
    v.push(3);
    // arg 1: string "session-0001"
    v.push(6);
    v.push(12);
    for c in "session-0001".bytes() {
        v.push(c as i64);
    }
    // arg 2: int
    v.push(5);
    v.push(12345);
    // arg 3: object {"a": true, "bb": "xy"}
    v.push(9);
    v.push(2);
    v.push(1);
    v.push(97);
    v.push(4);
    v.push(2);
    v.push(98);
    v.push(98);
    v.push(6);
    v.push(2);
    v.push(120);
    v.push(121);
    v[2] = v.len() as i64;
    v
}

fn perf(reps: usize) {
    let inp = typical_call();
    let mut out = vec![0i64; inp.len() + 16];
    let st = sol::m3_call(&inp, &mut out);
    if st != inp.len() as i64 || out[..inp.len()] != inp[..] {
        eprintln!("perf: typical call not accepted/echoed, status {}", st);
        std::process::exit(1);
    }
    // Warm-up.
    for _ in 0..(reps / 10).max(100) {
        black_box(sol::m3_call(black_box(&inp), &mut out));
    }
    let t = Instant::now();
    for _ in 0..reps {
        black_box(sol::m3_call(black_box(&inp), &mut out));
    }
    let ns = t.elapsed().as_nanos() as f64 / reps as f64;
    println!("m3_call_ns={:.1} words={} reps={}", ns, inp.len(), reps);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && args[1] == "perf" {
        let reps: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200000);
        perf(reps);
        return;
    }
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut w = io::BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let mut it = line.split_whitespace();
        let kind = match it.next() {
            Some(k) => k.to_string(),
            None => continue,
        };
        let inp: Vec<i64> = it.map(|t| t.parse::<i64>().expect("bad word")).collect();
        let mut out = vec![0i64; inp.len() + 16];
        let st = match kind.as_str() {
            "call" => sol::m3_call(&inp, &mut out),
            "resume" => sol::m3_resume(&inp, &mut out),
            "outcome" => sol::m3_outcome(&inp, &mut out),
            _ => {
                eprintln!("unknown kind {}", kind);
                std::process::exit(2);
            }
        };
        let mut s = format!("{}", st);
        if st > 0 {
            let n = (st as usize).min(out.len());
            for x in &out[..n] {
                s.push(' ');
                s.push_str(&x.to_string());
            }
        }
        writeln!(w, "{}", s).unwrap();
    }
    w.flush().unwrap();
}
