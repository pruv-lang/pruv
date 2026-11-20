// Driver: `driver <hand|gen> <sessions> <calls>` builds the state, runs the call loop and prints
// `ms=<loop time> sum=<checksum>`. Peak RSS is read from `/usr/bin/time -l` around one process.
// Build: rustc -O --edition 2021 driver.rs  (gen.rs is the pruvc output for p31k.pruv)
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
        let sids: Vec<String> = (0..n).map(|i| format!("sess-{:08}", i)).collect();
        let names: Vec<String> = (0..16).map(|i| format!("run-{:02}", i)).collect();
        let mut sessions = HashMap::with_capacity(n);
        for s in &sids {
            let mut runs = HashMap::new();
            for nm in &names[..4] {
                runs.insert(nm.clone(), true);
            }
            sessions.insert(s.clone(), $m::Session { used: 0, stopped: Vec::new(), runs });
        }
        let mut st = $m::St { sessions };
        let mut seed: u64 = 0x9e3779b97f4a7c15;
        let mut sum: i64 = 0;
        let t = Instant::now();
        for c in 0..$calls {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let sid = sids[(seed >> 33) as usize % n].as_str();
            if c & 15 == 0 {
                sum += $m::halt(&mut st, sid, names[(seed >> 20) as usize & 15].as_str());
            } else {
                sum += $m::charge(&mut st, sid, (c & 7) as i64);
            }
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let mut used = 0i64;
        let mut stopped = 0usize;
        for v in st.sessions.values() {
            used += v.used;
            stopped += v.stopped.len();
        }
        println!("ms={:.2} sum={} used={} stopped={}", ms, sum, used, stopped);
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
