#![allow(dead_code, unused_imports, unused_variables)]
// Pruv side of run_compiler_perf.py: @SOL@ is the compiled root (m6_test/root.pruv + perf_extra.pruv).
// Input: `op n args..` per line. Output: `ns <elapsed ns of the whole n-call loop> <checksum>`.
#[path = "@SOL@"] mod sol;
use std::io::{self, BufRead};
use std::time::Instant;

fn main() {
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let mut it = line.split_whitespace();
        let op = match it.next() { Some(o) => o.to_string(), None => continue };
        let w: Vec<i64> = it.map(|x| x.parse().unwrap()).collect();
        let t = Instant::now();
        let c = match op.as_str() {
            "p_iso" => sol::p_iso(w[0], w[1]),
            "p_ref" => sol::p_refusal(w[0], w[1], w[2], w[3], w[4], w[5]),
            "p_int" => sol::p_int(w[0], w[1]),
            "p_amt" => sol::p_amt(w[0], w[1], w[2]),
            "p_wr" => sol::p_wr(w[0], w[1]),
            _ => panic!("unknown op {}", op),
        };
        println!("ns {} {}", t.elapsed().as_nanos(), c);
    }
}
