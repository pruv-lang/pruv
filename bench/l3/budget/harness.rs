#![allow(dead_code, unused_imports, unused_variables)]
#[path="@SOL@"] mod sol;
// Links the generated Pruv module `sol` (m34_test/root_m4.pruv) to the budget host (budget_host).
//   harness q --prog <program.jsonl>   scenarios JSONL on stdin -> observations JSONL on stdout
//   harness perf <n> <reps>            one JSON line per measure (ns per op, min over reps after one warm-up)
use std::hint::black_box;
use std::time::Instant;
use budget_host::{empty_fire_seg, enc_obj, session_v, Core, ResidentState, World, J};

struct G;
impl Core for G {
    fn init(&self, st: &mut Vec<i64>) -> i64 {
        sol::m4_init(st)
    }
    fn grow(&self, prev: &Vec<i64>, next: &mut Vec<i64>) -> i64 {
        sol::m4_grow(prev, next)
    }
    fn op(&self, st: &mut Vec<i64>, inp: &[i64], out: &mut Vec<i64>) -> i64 {
        sol::m4_op(st, inp, out)
    }
}

// One warm-up run, then the minimum over `reps` runs; each run returns its own elapsed nanoseconds.
fn best<F: FnMut() -> u128>(reps: usize, mut f: F) -> u128 {
    f();
    let mut b = u128::MAX;
    for _ in 0..reps.max(1) {
        b = b.min(f());
    }
    b
}

fn line(measure: &str, n: usize, reps: usize, ns: u128, extra: &str) {
    println!("{{\"measure\":\"{}\",\"n\":{},\"reps\":{},\"ns_per_op\":{:.1}{}}}", measure, n, reps, ns as f64 / n.max(1) as f64, extra);
}

fn perf(n: usize, reps: usize) {
    // (a) timer_cycle: n x (tset then tclear), scheduler only, no trace
    let ns = best(reps, || {
        let mut w = World::new(false);
        let t = Instant::now();
        for i in 0..n {
            w.tset("x", black_box((i % 1000 + 1) as f64), true);
            w.tclear("x");
        }
        let e = t.elapsed().as_nanos();
        black_box(w.active_count());
        e
    });
    line("timer_cycle", n, reps, ns, "");

    // (b) timer_fire: n timers with distinct delays, one advance fires all (each runs an empty fire segment)
    let segs: Vec<J> = (0..n).map(|i| empty_fire_seg(&format!("k{}", i))).collect();
    let ns = best(reps, || {
        let mut w = World::new(false);
        w.set_program(segs.clone());
        for i in 0..n {
            w.tset(&format!("k{}", i), (i + 1) as f64, false);
        }
        let t = Instant::now();
        w.advance_by(black_box((n + 1) as f64)).expect("advance");
        let e = t.elapsed().as_nanos();
        assert_eq!(w.segs_left(), 0, "timer_fire: not every timer fired");
        e
    });
    line("timer_fire", n, reps, ns, "");

    // (c) store_find: n stored sessions, n lookups hitting different ids, result encoded and consumed
    let ids: Vec<String> = (0..n).map(|i| format!("s{}", (i * 7919) % n.max(1))).collect();
    let ns = best(reps, || {
        let mut w = World::new(false);
        for i in 0..n {
            w.put_session(session_v(&format!("s{}", i), i as f64)).expect("put");
        }
        let t = Instant::now();
        for id in &ids {
            black_box(w.find_session_enc(black_box(id)));
        }
        t.elapsed().as_nanos()
    });
    line("store_find", n, reps, ns, "");

    // (c') store_find breakdown: lookup only (id -> UTF-16 key, index probe; the non-tracing path) and the JV encode
    // of an already found session (labels assigned in an untimed pass first) plus dropping the J tree.
    let ns = best(reps, || {
        let mut w = World::new(false);
        for i in 0..n {
            w.put_session(session_v(&format!("s{}", i), i as f64)).expect("put");
        }
        let t = Instant::now();
        for id in &ids {
            black_box(w.find_session(black_box(id)));
        }
        t.elapsed().as_nanos()
    });
    line("store_find_lookup", n, reps, ns, "");
    let ns = best(reps, || {
        let mut w = World::new(false);
        for i in 0..n {
            w.put_session(session_v(&format!("s{}", i), i as f64)).expect("put");
        }
        black_box(w.encode_all_sessions());
        let hs: Vec<usize> = ids.iter().map(|id| w.find_session(id).expect("stored")).collect();
        let t = Instant::now();
        for &h in &hs {
            black_box(w.heap.enc(&budget_host::V::Sess(black_box(h))));
        }
        t.elapsed().as_nanos()
    });
    line("store_find_enc", n, reps, ns, "");

    // (d) store_save: n budget write-through saves (each sets a new budget) (labels s1..sn assigned by an untimed encode)
    let ns = best(reps, || {
        let mut w = World::new(false);
        for i in 0..n {
            w.put_session(session_v(&format!("s{}", i), 0.0)).expect("put");
        }
        black_box(w.encode_all_sessions());
        let raws: Vec<J> = (0..n)
            .map(|i| {
                let b = J::Obj(vec![
                    (budget_host::ku("bref"), J::s(&format!("b{}", i + 1))),
                    (budget_host::ku("v"), enc_obj(vec![("used", J::int(i as i64 + 1))])),
                ]);
                let v = enc_obj(vec![("id", J::s(&format!("s{}", i))), ("budget", b)]);
                J::Obj(vec![(budget_host::ku("sref"), J::s(&format!("s{}", i + 1))), (budget_host::ku("v"), v)])
            })
            .collect();
        let t = Instant::now();
        for r in &raws {
            w.scan(black_box(r)).expect("save");
        }
        let e = t.elapsed().as_nanos();
        let after = w.encode_all_sessions();
        let saved = after.iter().filter(|j| budget_host::json_text(j).contains("\"used\"")).count();
        assert_eq!(saved, n, "store_save: not every budget was written");
        e
    });
    line("store_save", n, reps, ns, "");

    // (e) lane: n session_put ops through ResidentState over the M4 Pruv entries, starting small so that Grow happens
    let ops: Vec<Vec<i64>> = (0..n)
        .map(|i| {
            let key = format!("s{}", i);
            let mut v: Vec<i64> = vec![1, key.len() as i64];
            v.extend(key.bytes().map(|b| b as i64));
            v.push(8);
            v.extend_from_slice(&[11, 22, 33, 44, 55, 66, 77, 88]);
            v
        })
        .collect();
    let mut grows = 0u64;
    let mut st_len = 0usize;
    let ns = best(reps, || {
        let mut rs = ResidentState::new(G, 256).expect("init");
        let t = Instant::now();
        for op in &ops {
            let r = rs.step(black_box(op)).expect("lane step");
            black_box(r.len());
        }
        let e = t.elapsed().as_nanos();
        grows = rs.grows;
        st_len = rs.st_len();
        e
    });
    line("lane", n, reps, ns, &format!(",\"grows\":{},\"final_st_len\":{}", grows, st_len));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("q") => {
            let mut prog: Option<String> = None;
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--prog" {
                    prog = args.get(i + 1).cloned();
                    i += 2;
                } else {
                    i += 1;
                }
            }
            match prog {
                Some(p) => {
                    if let Err(e) = budget_host::run_q(&p) {
                        eprintln!("harness q: {}", e);
                        std::process::exit(1);
                    }
                }
                None => {
                    eprintln!("usage: harness q --prog <program.jsonl> < scenarios.jsonl");
                    std::process::exit(2);
                }
            }
        }
        Some("perf") => {
            let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let reps: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
            if n == 0 || reps == 0 {
                eprintln!("usage: harness perf <n> <reps>");
                std::process::exit(2);
            }
            perf(n, reps);
        }
        _ => {
            eprintln!("usage: harness q --prog <program.jsonl> < scenarios.jsonl | harness perf <n> <reps>");
            std::process::exit(2);
        }
    }
}
