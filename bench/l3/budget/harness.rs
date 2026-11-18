#![allow(dead_code, unused_imports, unused_variables)]
#[path="@SOL@"] mod sol;
// Links the generated Pruv module `sol` (m34_test/root_m4.pruv) to the budget host (budget_host).
//   harness q --prog <program.jsonl>   scenarios JSONL on stdin -> observations JSONL on stdout
//   harness perf <n> <reps>            one JSON line per measure (ns per op, min over reps after one warm-up)
//   harness grow [n]                   growth checks: Q2/Q3 growth, allocation-failure injection, copy accounting
use std::hint::black_box;
use std::time::Instant;
use budget_host::{empty_fire_seg, enc_obj, session_v, Core, ResidentState, World, GROW_ALLOC_ERR, J};

struct G;
impl Core for G {
    fn init(&self, st: &mut [i64]) -> i64 {
        sol::m4_init(st)
    }
    fn grow(&self, prev: &[i64], next: &mut [i64]) -> i64 {
        sol::m4_grow(prev, next)
    }
    fn op(&self, st: &mut [i64], inp: &[i64], out: &mut [i64]) -> i64 {
        sol::m4_op(st, inp, out)
    }
    fn copied(&self, st: &[i64]) -> i64 {
        sol::m4_copied(st)
    }
}

// session_put wire op: key `n, packed words(n)` (four big-endian u16 lanes per i64 word, zero tail lanes,
// bud_keys.pruv; packed once here, the host ingress; ASCII so each byte is one unit), then `w` record words.
fn put_op(key: &str, w: usize, seed: i64) -> Vec<i64> {
    let mut v: Vec<i64> = vec![1, key.len() as i64];
    v.extend(key.as_bytes().chunks(4).map(|c| {
        let mut x: u64 = 0;
        for l in 0..4 {
            x = (x << 16) | (c.get(l).copied().unwrap_or(0) as u64);
        }
        x as i64
    }));
    v.push(w as i64);
    for j in 0..w {
        v.push(seed.wrapping_mul(31).wrapping_add(j as i64 + 11));
    }
    v
}

fn observe(st: &[i64]) -> Vec<i64> {
    let mut out = vec![0i64; 1 << 16];
    loop {
        let r = sol::m4_observe(&st.to_vec(), &mut out);
        if r == 0 {
            return out;
        }
        assert_eq!(r, 3, "observe status");
        let l = out.len() * 2;
        out.resize(l, 0);
    }
}

// Growth checks (L3 P13b), exit 1 on any failure; one JSON line of byte/copy accounting.
//   Q2: n fresh keys from a 256-word start (index and arena grow together).
//   Q3: the same keys rewritten with longer records (arena-driven growth, dead words dropped by the copy).
//   For both: every grow allocates exactly the N Pruv asked for, N >= 2*len (geometric), total allocated and copied
//   words stay within a constant of the final length (amortized linear), and an injected allocation failure at every
//   grow point leaves the state words and the observation unchanged, after which the same op succeeds and the run
//   ends in the same state as the run without injections.
fn grow_check(n: usize) {
    let mut ops: Vec<Vec<i64>> = (0..n).map(|i| put_op(&format!("s{}", i), 8, i as i64)).collect();
    let q2 = ops.len();
    ops.extend((0..n).map(|i| put_op(&format!("s{}", i), 40 + (i % 80), 7 + i as i64)));
    let mut fails = 0usize;
    let mut check = |ok: bool, what: &str| {
        if !ok {
            eprintln!("grow_check: {}", what);
            fails += 1;
        }
    };
    // clean run
    let mut rs = ResidentState::new(G, 256).expect("init");
    let mut lens: Vec<usize> = vec![rs.st_len()];
    let mut q2_grows = 0u64;
    let mut grew_at: Vec<usize> = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        let before = rs.st_len();
        rs.step(op).expect("clean step");
        if rs.st_len() != before {
            lens.push(rs.st_len());
            grew_at.push(i);
        }
        if i + 1 == q2 {
            q2_grows = rs.grows;
        }
    }
    let final_len = rs.st_len() as u64;
    for w in lens.windows(2) {
        check(w[1] >= 2 * w[0], "grow step below 2*len");
    }
    check(rs.grows as usize == lens.len() - 1, "one buffer per grow");
    // exactly N: the host adds the requested N to grow_alloc_words, so the buffer lengths it installed must sum to it
    check(lens[1..].iter().map(|&l| l as u64).sum::<u64>() == rs.grow_alloc_words, "installed lengths differ from the requested N");
    check(q2_grows > 0, "Q2 grows");
    // Q3 must grow when its record words alone exceed the whole buffer left by Q2 (the arena cannot hold them even
    // with every dead word reclaimed); with fewer words it may fit, so no growth is required then.
    let q3_words: usize = (0..n).map(|i| 40 + (i % 80)).sum(); // record words only (the Q3 put_op w values)
    let len_q2 = lens[..=q2_grows as usize].last().copied().unwrap_or(0);
    if q3_words > len_q2 {
        check(rs.grows > q2_grows, "Q3 input exceeds the buffer but Q3 did not grow");
    }
    check(rs.grow_alloc_words <= 2 * final_len, "allocated words exceed 2 * final length");
    check(rs.grow_copy_words <= 2 * final_len, "copied words exceed 2 * final length");
    // copy_words meaning (bud_store::copy_words): 20 words per entry up to the entry top (roots included) plus the
    // arena words in use; the header is not counted. Known states: the fresh store (3 roots, empty arena) copies
    // exactly 60, and each extra put of the same shape adds the same number of words, at least 20 + its record.
    let copy_of = |k: usize| -> i64 {
        let mut s = ResidentState::new(G, 4096).expect("init");
        for op in &ops[..k] {
            s.step(op).expect("known-state step");
        }
        let mut fresh = vec![0i64; 2 * s.st_len()];
        let g = sol::m4_grow(&s.state().to_vec(), &mut fresh);
        if g != 0 { -1 } else { sol::m4_copied(&fresh) }
    };
    let (c0, c1, c2) = (copy_of(0), copy_of(1), copy_of(2));
    check(c0 == 60, "fresh store copy count is not 60");
    check(c1 - c0 == c2 - c1 && c1 - c0 >= 20 + 8, "per-put copy count is not constant or below 20 + record words");
    check(rs.grow_copy_words >= 60 * rs.grows, "copied words below the roots per grow");
    let clean_obs = observe(rs.state());
    let clean_words = rs.state().to_vec();
    // injected allocation failure before every op: an op that needs to grow fails with the old state kept. The run is
    // deterministic, so the failing ops are exactly the clean run's grow points; the state and observation are
    // snapshotted there only (a snapshot per op would make the check quadratic).
    let mut inj = ResidentState::new(G, 256).expect("init");
    let mut injected = 0u64;
    for (i, op) in ops.iter().enumerate() {
        let at = grew_at.binary_search(&i).is_ok();
        let (w0, o0) = if at { let w = inj.state().to_vec(); let o = observe(&w); (w, o) } else { (Vec::new(), Vec::new()) };
        inj.fail_allocs = 1;
        match inj.step(op) {
            Ok(_) => check(!at, "a grow point did not try to allocate"),
            Err(e) => {
                check(at, "an allocation outside the clean run's grow points");
                check(e == GROW_ALLOC_ERR, "unexpected step error");
                injected += 1;
                check(inj.state() == &w0[..], "state words changed by a failed grow");
                check(observe(inj.state()) == o0, "observation changed by a failed grow");
                inj.fail_allocs = 0;
                inj.step(op).expect("retry after a refused allocation");
            }
        }
        inj.fail_allocs = 0;
    }
    check(injected == rs.grows, "one injected failure per grow point");
    check(inj.state() == &clean_words[..], "injected run ends in a different state");
    check(observe(inj.state()) == clean_obs, "injected run observes differently");
    println!(
        "{{\"measure\":\"grow_check\",\"ops\":{},\"q2_grows\":{},\"grows\":{},\"final_len\":{},\"alloc_words\":{},\"copy_words\":{},\"alloc_per_op\":{:.2},\"copy_per_op\":{:.2},\"grow_ns\":{},\"injected\":{},\"fails\":{}}}",
        ops.len(), q2_grows, rs.grows, final_len, rs.grow_alloc_words, rs.grow_copy_words,
        rs.grow_alloc_words as f64 / ops.len() as f64, rs.grow_copy_words as f64 / ops.len() as f64,
        rs.grow_ns, injected, fails
    );
    if fails != 0 {
        std::process::exit(1);
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

    // (c) store_find: n stored sessions, n lookups hitting different ids, result encoded as text into a reused buffer
    let ids: Vec<String> = (0..n).map(|i| format!("s{}", (i * 7919) % n.max(1))).collect();
    let ns = best(reps, || {
        let mut w = World::new(false);
        for i in 0..n {
            w.put_session(session_v(&format!("s{}", i), i as f64)).expect("put");
        }
        let mut out = String::with_capacity(128);
        let t = Instant::now();
        for id in &ids {
            w.find_session_text(black_box(id), &mut out);
            black_box(out.len());
        }
        t.elapsed().as_nanos()
    });
    line("store_find", n, reps, ns, "");

    // (c') store_find breakdown: lookup only (id -> UTF-16 key, index probe; the lookup helper only, not the whole non-tracing exec_item) and the JV encode
    // of an already found session as text into a reused buffer (labels assigned in an untimed pass first).
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
        let mut out = String::with_capacity(128);
        let t = Instant::now();
        for &h in &hs {
            out.clear();
            w.heap.enc_into(&budget_host::V::Sess(black_box(h)), &mut out);
            black_box(out.len());
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
    let ops: Vec<Vec<i64>> = (0..n).map(|i| put_op(&format!("s{}", i), 8, i as i64)).collect();
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
        Some("grow") => {
            let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3000);
            grow_check(n);
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
