// Driver: `driver <hand|gen> <clocks> <calls>` builds the state, runs the call loop and prints
// `ms=<loop time> sum=<checksum>`. Peak RSS is read from `/usr/bin/time -l` around one process.
// Build: rustc -O --edition 2021 --crate-type rlib --crate-name p31n_facts p31n_facts.rs -o libp31n_facts.rlib
//        rustc -O --edition 2021 --extern p31n_facts=libp31n_facts.rlib driver.rs
// (gen.rs is the pruvc output for p31n.pruv with --rust-extern p31n_facts=libp31n_facts.rlib)
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
        let sids: Vec<String> = (0..n).map(|i| format!("clk-{:08}", i)).collect();
        let names: Vec<String> = (0..16).map(|i| format!("run-{:02}", i)).collect();
        let owners: Vec<String> = (0..2).map(|i| format!("owner-{}", i)).collect();
        let mut clocks = HashMap::with_capacity(n);
        for (i, s) in sids.iter().enumerate() {
            let mut runs = HashMap::new();
            for nm in &names[..4] {
                runs.insert(nm.clone(), 1i64);
            }
            clocks.insert(s.clone(), $m::Clock { owner: owners[i & 1].clone(), since_ms: 0, runs });
        }
        let mut st = $m::St { clocks, out: Vec::with_capacity(16) };
        let mut seed: u64 = 0x9e3779b97f4a7c15;
        let mut sum: i64 = 0;
        let t = Instant::now();
        for c in 0..$calls {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let k = (seed >> 33) as usize % n;
            let sid = sids[k].as_str();
            if c & 15 == 0 {
                sum += $m::halt(&mut st, sid, owners[(seed >> 20) as usize & 1].as_str());
                sum += st.out.len() as i64;
            } else {
                sum += $m::tick(&mut st, sid, names[(seed >> 20) as usize & 15].as_str(), c as i64);
            }
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let mut total = 0i64;
        let mut runs = 0usize;
        for v in st.clocks.values() {
            runs += v.runs.len();
            for x in v.runs.values() {
                total += *x;
            }
        }
        println!("ms={:.2} sum={} total={} runs={}", ms, sum, total, runs);
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
