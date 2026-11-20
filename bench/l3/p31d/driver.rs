// Driver: `driver <hand|gen> <ids> <calls>` builds the state, runs the call loop and prints
// `ms=<loop time> sum=<checksum>`. Peak RSS is read from `/usr/bin/time -l` around one process.
// Build: rustc -O --edition 2021 driver.rs  (gen.rs is the pruvc output for p31d.pruv)
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
        let bids: Vec<String> = (0..n).map(|i| format!("clk-{:08}", i)).collect();
        let mut sessions = HashMap::with_capacity(n);
        let mut clocks = HashMap::with_capacity(n);
        for (i, s) in sids.iter().enumerate() {
            sessions.insert(s.clone(), $m::Budget { limit_ms: 500 + (i as i64 % 1000) * 50, active_ms: 0 });
        }
        for b in &bids {
            clocks.insert(b.clone(), $m::Clock { since_ms: 0, armed: true });
        }
        let mut st = $m::St { sessions, clocks };
        let mut seed: u64 = 0x9e3779b97f4a7c15;
        let mut sum: i64 = 0;
        let t = Instant::now();
        for c in 0..$calls {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let k = (seed >> 33) as usize % n;
            let now = ((c / 64) % 1_000_000) as i64;
            if c & 15 == 0 {
                let r = $m::poll(&mut st, sids[k].as_str());
                sum += r.status + r.effects.len() as i64;
                for e in &r.effects {
                    sum += e.tag + (e.argument & 1);
                }
            } else {
                sum += $m::refresh(&mut st, sids[k].as_str(), bids[(k + c as usize) % n].as_str(), now);
            }
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let mut active = 0i64;
        let mut armed = 0usize;
        for v in st.sessions.values() {
            active += v.active_ms;
        }
        for v in st.clocks.values() {
            armed += v.armed as usize;
        }
        println!("ms={:.2} sum={} active={} armed={}", ms, sum, active, armed);
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
