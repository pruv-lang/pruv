// Driver: `driver <hand|gen> <clocks> <calls>` builds the state, runs the call loop and prints
// `ms=<loop time> sum=<checksum> ...`. Peak RSS is read from `/usr/bin/time -l` around one process.
// Build: rustc -O --edition 2021 driver.rs   (gen.rs is the pruvc output for p31r.pruv)
use std::collections::HashMap;
use std::time::Instant;

#[path = "hand.rs"]
mod hand;
#[path = "gen.rs"]
#[allow(dead_code, unused_parens, unused_mut, non_snake_case, unused_variables)]
mod gen;

macro_rules! run {
    ($m:ident, $n:expr, $calls:expr) => {{
        let n: usize = $n;
        let a: Vec<String> = (0..n).map(|i| format!("clk-{:08}-a", i)).collect();
        let b: Vec<String> = (0..n).map(|i| format!("clk-{:08}-b", i)).collect();
        let mut clocks = HashMap::with_capacity(n);
        for (i, s) in a.iter().enumerate() {
            clocks.insert(s.clone(), $m::Clock { armed: i & 1 == 0, used: i as i64, note: format!("note-{}", i) });
        }
        let mut st = $m::St { clocks };
        let mut seed: u64 = 0x9e3779b97f4a7c15;
        let mut sum: i64 = 0;
        let t = Instant::now();
        for c in 0..$calls {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let k = (seed >> 33) as usize % n;
            if c & 15 == 0 {
                let e = $m::end(&mut st, a[k].as_str()) + $m::end(&mut st, b[k].as_str());
                sum += e;
                st.clocks.insert(a[k].clone(), $m::Clock { armed: c & 2 == 0, used: e & 1023, note: String::new() });
            } else {
                let r = $m::rekey(&mut st, a[k].as_str(), b[k].as_str());
                let r = if r == 1 { $m::rekey(&mut st, b[k].as_str(), a[k].as_str()) + 8 } else { r };
                sum += r;
            }
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let mut total = 0i64;
        let mut armed = 0usize;
        for v in st.clocks.values() {
            total += v.used;
            armed += v.armed as usize + v.note.len();
        }
        println!("ms={:.2} sum={} total={} n={} armed={}", ms, sum, total, st.clocks.len(), armed);
    }};
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a[2].parse().unwrap();
    let calls: u64 = a[3].parse().unwrap();
    match a[1].as_str() {
        "hand" => run!(hand, n, calls),
        _ => run!(gen, n, calls),
    }
}
