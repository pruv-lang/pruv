#![allow(dead_code, unused_imports)]
// P10H harness: `gates` and `perf <rounds> [label]`. The compiled Pruv module is @SOL@; the host is hostfact_host.
#[path = "@SOL@"] mod sol;
use hostfact_host as h;
use std::cell::Cell;
use std::hint::black_box;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;

type Trace = Vec<h::Event>;

// ---------------------------------------------------------------- hand lanes (same semantics, same host facts)
// Same `requires` as the Pruv bh_halt; short-circuit order matches (len(st) >= 3 before indexing).
fn check_halt_requires(st: &[i64], out: &[i64]) {
    if !(st.len() >= 3 && 0 <= st[0] && st[0] <= 1000000000000 && 0 <= st[1] && st[1] <= 1000000000000) {
        panic!("pruv: requires violated: bh_halt st (len >= 3, 0 <= st[0], st[1] <= 1000000000000)");
    }
    if !(out.len() <= 100000000) {
        panic!("pruv: requires violated: bh_halt len(out) <= 100000000");
    }
}

fn hand_bh_halt(st: &mut [i64], session: i64, out: &mut [i64]) -> i64 {
    check_halt_requires(st, out);
    st[0] += st[1];
    st[1] = 0;
    let v = h::running_ids(session, st[0]);
    let total = v.len() as i64;
    if total < 2 || v[0] != 0 || v[1] != total - 2 {
        return 2;
    }
    if v[2..].iter().any(|&x| x <= 0) {
        return 2;
    }
    if 2 * total > out.len() as i64 {
        return 3;
    }
    st[2] = 1;
    out[0] = total + 1;
    out[1] = 1;
    out[2] = 2;
    out[3] = 3;
    let mut p = 4;
    for &id in &v[2..] {
        out[p] = 4;
        out[p + 1] = id;
        p += 2;
    }
    0
}

// Mutant: queries before SETTLE (the order bug the gate must catch).
fn mutant_bh_halt(st: &mut [i64], session: i64, out: &mut [i64]) -> i64 {
    check_halt_requires(st, out);
    let v = h::running_ids(session, st[0]);
    st[0] += st[1];
    st[1] = 0;
    let total = v.len() as i64;
    if total < 2 || v[0] != 0 || v[1] != total - 2 || v[2..].iter().any(|&x| x <= 0) {
        return 2;
    }
    if 2 * total > out.len() as i64 {
        return 3;
    }
    st[2] = 1;
    out[0] = total + 1;
    out[1] = 1;
    out[2] = 2;
    out[3] = 3;
    let mut p = 4;
    for &id in &v[2..] {
        out[p] = 4;
        out[p + 1] = id;
        p += 2;
    }
    0
}

fn hand_bh_close(session: i64) -> i64 {
    match h::is_quiet(session) {
        0 => 10,
        1 => 11,
        _ => 2,
    }
}

fn hand_bh_release(session: i64) -> i64 {
    match h::release_held(session) {
        1 => 0,
        0 => 1,
        _ => 2,
    }
}

fn hand_rq_find(s_n: i64, m_n: i64) -> i64 {
    // same `requires` as the Pruv entry
    if !(0 <= s_n && s_n <= 1000000000 && 0 <= m_n && m_n <= 1000000000) {
        panic!("pruv: requires violated: rq_find(s_n={}, m_n={})", s_n, m_n);
    }
    let mut s = 0;
    while s < s_n {
        let _title = h::title_obs(s);
        let mut m = 0;
        while m < m_n {
            let it = h::next_item(s, m);
            let t = h::is_temporary(it);
            if t != 0 && t != 1 {
                return -2;
            }
            if t == 1 {
                return it;
            }
            m += 1;
        }
        s += 1;
    }
    -1
}

// Control: same loops, the host calls replaced by opaque inline arithmetic (splits call cost from loop work).
fn control_rq_find(s_n: i64, m_n: i64) -> i64 {
    let mut acc = 0i64;
    let mut s = 0;
    while s < s_n {
        acc = acc.wrapping_add(black_box(s + 1));
        let mut m = 0;
        while m < m_n {
            let it = black_box(s * 16 + m + 1);
            let t = black_box(it >> 62);
            if t != 0 && t != 1 {
                return -2;
            }
            if t == 1 {
                return it;
            }
            m += 1;
        }
        s += 1;
    }
    black_box(acc);
    -1
}

// ---------------------------------------------------------------- entry adapter with the re-entrancy guard
thread_local! { static ACTIVE: Cell<bool> = const { Cell::new(false) }; static LANES: Cell<(u32, u32)> = const { Cell::new((0, 0)) }; }

struct Guard;
impl Guard {
    fn enter() -> Option<Guard> {
        if ACTIVE.with(|a| a.replace(true)) { None } else { Some(Guard) }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.with(|a| a.set(false));
    }
}

const REFUSED_REENTRY: i64 = 9;

fn a_halt(st: &mut Vec<i64>, session: i64, out: &mut Vec<i64>) -> i64 {
    match Guard::enter() { None => REFUSED_REENTRY, Some(_g) => sol::bh_halt(st, session, out) }
}
fn a_hand_halt(st: &mut Vec<i64>, session: i64, out: &mut Vec<i64>) -> i64 {
    match Guard::enter() { None => REFUSED_REENTRY, Some(_g) => hand_bh_halt(st, session, out) }
}
fn a_mutant_halt(st: &mut Vec<i64>, session: i64, out: &mut Vec<i64>) -> i64 {
    match Guard::enter() { None => REFUSED_REENTRY, Some(_g) => mutant_bh_halt(st, session, out) }
}
fn a_close(st: &Vec<i64>, session: i64) -> i64 {
    match Guard::enter() { None => REFUSED_REENTRY, Some(_g) => sol::bh_close(st, session) }
}
fn a_release(session: i64) -> i64 {
    match Guard::enter() { None => REFUSED_REENTRY, Some(_g) => sol::bh_release(session) }
}
fn a_find(s_n: i64, m_n: i64) -> i64 {
    match Guard::enter() { None => REFUSED_REENTRY, Some(_g) => sol::rq_find(s_n, m_n) }
}

fn cb_pruv() -> i64 {
    let mut st = vec![0i64, 0, 0];
    let mut out = vec![0i64; 8];
    a_halt(&mut st, 2, &mut out)
}
fn cb_hand() -> i64 {
    let mut st = vec![0i64, 0, 0];
    let mut out = vec![0i64; 8];
    a_hand_halt(&mut st, 2, &mut out)
}

// ---------------------------------------------------------------- observation helpers
type HaltFn = fn(&mut Vec<i64>, i64, &mut Vec<i64>) -> i64;

#[derive(Debug, PartialEq, Clone)]
struct Obs {
    ret: i64,
    st: Vec<i64>,
    out: Vec<i64>,
    trace: Trace,
}

fn halt_case(f: HaltFn, st0: &[i64], running: Vec<i64>, out_len: usize, reenter: Option<fn() -> i64>) -> Obs {
    h::reset();
    h::set_running(running);
    h::set_reenter(reenter);
    let mut st = st0.to_vec();
    let mut out = vec![-1i64; out_len];
    let ret = f(&mut st, 1, &mut out);
    Obs { ret, st, out, trace: h::take_trace() }
}

/// Run a case on the Pruv lane and the hand lane; the two must agree exactly. Returns the Pruv observation.
fn halt_both(st0: &[i64], running: &[i64], out_len: usize, reenter: Option<(fn() -> i64, fn() -> i64)>) -> Result<Obs, String> {
    let p = halt_case(a_halt, st0, running.to_vec(), out_len, reenter.map(|r| r.0));
    let q = halt_case(a_hand_halt, st0, running.to_vec(), out_len, reenter.map(|r| r.1));
    lane_eq("halt", p, q)
}

fn lane_eq<T: PartialEq + std::fmt::Debug>(what: &str, p: T, q: T) -> Result<T, String> {
    let same = p == q;
    LANES.with(|c| {
        let (n, bad) = c.get();
        c.set((n + 1, bad + (!same) as u32));
    });
    if same { Ok(p) } else { Err(format!("lane mismatch in {}: pruv {:?} hand {:?}", what, p, q)) }
}

fn expect<T: PartialEq + std::fmt::Debug>(what: &str, got: T, want: T) -> Result<(), String> {
    if got == want { Ok(()) } else { Err(format!("{}: got {:?} want {:?}", what, got, want)) }
}

fn untouched(o: &Obs) -> Result<(), String> {
    expect("halted flag", o.st[2], 0)?;
    expect("out untouched", o.out.iter().all(|&x| x == -1), true)
}

const RUN3: [i64; 5] = [0, 3, 7, 7, 9];

fn gate_halt_order() -> Result<(), String> {
    let o = halt_both(&[5, 3, 0], &RUN3, 32, None)?;
    expect("ret", o.ret, 0)?;
    expect("st", o.st.clone(), vec![8, 0, 1])?;
    expect("trace", o.trace.clone(), vec![(h::T_RUNNING, 1, 8)])?;
    // effects: count, halted, cancelRecovery, stopMain, then (4, id) per id in order, duplicates included
    let mut want = vec![6, 1, 2, 3, 4, 7, 4, 7, 4, 9];
    want.resize(32, -1);
    expect("out", o.out, want)
}

fn order_ok(trace: &Trace, settled: i64) -> bool {
    trace.first() == Some(&(h::T_RUNNING, 1, settled))
}

fn gate_halt_mutant() -> Result<(), String> {
    let good = halt_case(a_halt, &[5, 3, 0], RUN3.to_vec(), 32, None);
    let bad = halt_case(a_mutant_halt, &[5, 3, 0], RUN3.to_vec(), 32, None);
    expect("pruv settled arg", order_ok(&good.trace, 8), true)?;
    expect("mutant caught (queried before settle)", order_ok(&bad.trace, 8), false)?;
    expect("mutant arg", bad.trace.clone(), vec![(h::T_RUNNING, 1, 5)])?;
    expect("traces differ", good.trace != bad.trace, true)
}

fn gate_refusal(name: &str, running: &[i64], out_len: usize, want: i64) -> Result<(), String> {
    let o = halt_both(&[5, 3, 0], running, out_len, None)?;
    expect(&format!("{} ret", name), o.ret, want)?;
    untouched(&o)
}

fn gate_halt_small_out() -> Result<(), String> {
    gate_refusal("n3 out 9", &RUN3, 9, 3)?;
    gate_refusal("n3 out 3", &RUN3, 3, 3)?;
    gate_refusal("n3 out 0", &RUN3, 0, 3)?;
    let o = halt_both(&[5, 3, 0], &RUN3, 10, None)?;
    expect("exact capacity ret", o.ret, 0)
}

fn gate_close() -> Result<(), String> {
    for (answer, want) in [(0i64, 10i64), (1, 11), (7, 2), (-1, 2)] {
        let mut got = Vec::new();
        for lane in 0..2 {
            h::reset();
            h::set_quiet(answer);
            let st = vec![0i64, 0, 0];
            let r = if lane == 0 { a_close(&st, 4) } else { hand_bh_close(4) };
            got.push((r, h::take_trace()));
        }
        let (r, t) = lane_eq("close", got[0].clone(), got[1].clone())?;
        expect(&format!("close answer {}", answer), r, want)?;
        expect("close trace", t, vec![(h::T_QUIET, 4, 0)])?;
    }
    Ok(())
}

fn gate_release() -> Result<(), String> {
    for (answer, want) in [(1i64, 0i64), (0, 1), (7, 2)] {
        let mut got = Vec::new();
        for lane in 0..2 {
            h::reset();
            h::set_release(answer);
            let r = if lane == 0 { a_release(4) } else { hand_bh_release(4) };
            got.push((r, h::take_trace()));
        }
        let (r, t) = lane_eq("release", got[0].clone(), got[1].clone())?;
        expect(&format!("release answer {}", answer), r, want)?;
        expect("release trace", t, vec![(h::T_RELEASE, 4, 0)])?;
    }
    Ok(())
}

fn gate_reentry() -> Result<(), String> {
    let o = halt_both(&[5, 3, 0], &RUN3, 32, Some((cb_pruv as fn() -> i64, cb_hand as fn() -> i64)))?;
    expect("outer ret", o.ret, 0)?;
    expect("st", o.st.clone(), vec![8, 0, 1])?;
    // the callback ran inside the query and was refused with code 9
    expect("trace", o.trace.clone(), vec![(h::T_RUNNING, 1, 8), (h::T_REENTRY, REFUSED_REENTRY, 0)])?;
    expect("effects", o.out[..10].to_vec(), vec![6, 1, 2, 3, 4, 7, 4, 7, 4, 9])?;
    expect("guard cleared after call", ACTIVE.with(|a| a.get()), false)?;
    let again = halt_case(a_halt, &[1, 1, 0], RUN3.to_vec(), 32, None);
    expect("next call works", again.ret, 0)?;
    // the guard is cleared on unwind too
    std::panic::set_hook(Box::new(|_| {}));
    let r = catch_unwind(AssertUnwindSafe(|| {
        let _g = Guard::enter();
        panic!("unwind with guard held");
    }));
    let _ = std::panic::take_hook();
    expect("panic seen", r.is_err(), true)?;
    expect("guard cleared on unwind", ACTIVE.with(|a| a.get()), false)
}

/// Expected req trace, built independently of the host: per session title, per item next + temp; ends after the
/// is_temporary call of `stop` (s, m) when given.
fn req_trace(s_n: i64, m_n: i64, stop: Option<(i64, i64)>) -> Trace {
    let mut t = Vec::new();
    for s in 0..s_n {
        t.push((h::T_TITLE, s, 0));
        for m in 0..m_n {
            let it = s * 16 + m + 1;
            t.push((h::T_NEXT, s, m));
            t.push((h::T_TEMP, it, 0));
            if stop == Some((s, m)) {
                return t;
            }
        }
    }
    t
}

fn req_both(s_n: i64, m_n: i64, grid: Option<(i64, i64, i64)>) -> Result<(i64, Trace), String> {
    let mut got = Vec::new();
    for lane in 0..2 {
        h::reset();
        if let Some((s, m, a)) = grid {
            h::set_req_grid(s, m, a);
        }
        let r = if lane == 0 { a_find(s_n, m_n) } else { hand_rq_find(s_n, m_n) };
        got.push((r, h::take_trace()));
    }
    lane_eq("req", got[0].clone(), got[1].clone())
}

fn gate_req_first_match() -> Result<(), String> {
    let (r, t) = req_both(20, 10, Some((3, 4, 1)))?;
    expect("item id", r, 3 * 16 + 4 + 1)?;
    expect("trace", t, req_trace(20, 10, Some((3, 4))))
}

fn gate_req_invalid() -> Result<(), String> {
    let (r, t) = req_both(20, 10, Some((2, 1, 5)))?;
    expect("ret", r, -2)?;
    expect("trace", t, req_trace(20, 10, Some((2, 1))))
}

fn gate_req_nomatch() -> Result<(), String> {
    let (r, t) = req_both(7, 3, None)?;
    expect("ret", r, -1)?;
    expect("call count", t.len() as i64, 7 * (1 + 2 * 3))?;
    expect("trace", t, req_trace(7, 3, None))?;
    let (r0, t0) = req_both(0, 3, None)?;
    expect("empty grid ret", r0, -1)?;
    expect("empty grid calls", t0.len(), 0)
}

fn gate_lanes_equal() -> Result<(), String> {
    let (n, bad) = LANES.with(|c| c.get());
    expect("lane mismatches", bad, 0)?;
    expect("lane comparisons made", n > 0, true)
}

fn run_gate(name: &str, f: fn() -> Result<(), String>, fails: &mut u32) {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => println!("GATE {} ok", name),
        Ok(Err(d)) => { println!("GATE {} FAIL {}", name, d); *fails += 1; }
        Err(_) => { println!("GATE {} FAIL panic", name); *fails += 1; }
    }
}

fn gates() -> i32 {
    let mut fails = 0;
    run_gate("halt_order", gate_halt_order, &mut fails);
    run_gate("halt_mutant", gate_halt_mutant, &mut fails);
    run_gate("halt_bad_status", || gate_refusal("status", &[1, 0], 32, 2), &mut fails);
    run_gate("halt_bad_len", || gate_refusal("len", &[0, 2, 7], 32, 2), &mut fails);
    run_gate("halt_bad_id", || gate_refusal("id", &[0, 1, 0], 32, 2), &mut fails);
    run_gate("halt_small_out", gate_halt_small_out, &mut fails);
    run_gate("close", gate_close, &mut fails);
    run_gate("release", gate_release, &mut fails);
    run_gate("reentry", gate_reentry, &mut fails);
    run_gate("req_first_match", gate_req_first_match, &mut fails);
    run_gate("req_invalid", gate_req_invalid, &mut fails);
    run_gate("req_nomatch", gate_req_nomatch, &mut fails);
    run_gate("hand_lanes_equal", gate_lanes_equal, &mut fails);
    if fails > 0 { 1 } else { 0 }
}

// ---------------------------------------------------------------- perf
fn median(mut v: Vec<u128>) -> u128 {
    v.sort();
    v[v.len() / 2]
}

/// Interleaved timing: one untimed warm-up of every lane, then per round run the lanes in a rotated order
/// (start lane = round index mod lane count), timing each lane separately. Returns the per-lane medians.
fn bench_alt(rounds: usize, lanes: &mut [&mut dyn FnMut()]) -> Vec<u128> {
    let n = lanes.len();
    for lane in lanes.iter_mut() {
        lane(); // warm-up, not timed
    }
    let mut samples: Vec<Vec<u128>> = (0..n).map(|_| Vec::with_capacity(rounds)).collect();
    for r in 0..rounds {
        for k in 0..n {
            let i = (r + k) % n;
            let t = Instant::now();
            lanes[i]();
            samples[i].push(t.elapsed().as_nanos());
        }
    }
    samples.into_iter().map(median).collect()
}

fn emit(lane: &str, backend: &str, ns: u128, calls: u128) {
    println!("{{\"lane\":\"{}\",\"backend\":\"{}\",\"median_ns\":{},\"per_call_ns\":{:.3},\"calls\":{}}}",
             lane, backend, ns, ns as f64 / calls as f64, calls);
}

fn perf(rounds: usize, label: &str) -> i32 {
    const S: i64 = 20000;
    const M: i64 = 10;
    const CALLS: u128 = (S * (1 + 2 * M)) as u128; // 420000 host calls per no-match run
    h::reset();
    h::set_trace(false);
    if sol::rq_find(S, M) != -1 || hand_rq_find(S, M) != -1 || control_rq_find(S, M) != -1 {
        println!("PERF FAIL rq_find no-match answer");
        return 1;
    }
    let mut f_pr = || { black_box(sol::rq_find(black_box(S), black_box(M))); };
    let mut f_hd = || { black_box(hand_rq_find(black_box(S), black_box(M))); };
    let mut f_ct = || { black_box(control_rq_find(black_box(S), black_box(M))); };
    let med = bench_alt(rounds, &mut [&mut f_pr, &mut f_hd, &mut f_ct]);
    emit("rq_find_pruv", label, med[0], CALLS);
    emit("rq_find_hand", label, med[1], CALLS);
    emit("rq_find_control", label, med[2], CALLS);

    const REPS: u128 = 200000;
    h::set_running(RUN3.to_vec());
    // each lane owns its state/output vectors so the two closures never borrow the same data
    let mut st_p = vec![5i64, 3, 0];
    let mut out_p = vec![0i64; 16];
    let mut st_h = vec![5i64, 3, 0];
    let mut out_h = vec![0i64; 16];
    if sol::bh_halt(&mut st_p, 1, &mut out_p) != 0 || st_p != [8, 0, 1] {
        println!("PERF FAIL bh_halt answer");
        return 1;
    }
    let mut f_hp = || {
        for _ in 0..REPS {
            st_p[0] = black_box(5); st_p[1] = black_box(3); st_p[2] = 0;
            black_box(sol::bh_halt(&mut st_p, black_box(1), &mut out_p));
            black_box(&st_p);
            black_box(&out_p);
        }
    };
    let mut f_hh = || {
        for _ in 0..REPS {
            st_h[0] = black_box(5); st_h[1] = black_box(3); st_h[2] = 0;
            black_box(hand_bh_halt(&mut st_h, black_box(1), &mut out_h));
            black_box(&st_h);
            black_box(&out_h);
        }
    };
    let med = bench_alt(rounds, &mut [&mut f_hp, &mut f_hh]);
    emit("halt_pruv", label, med[0], REPS);
    emit("halt_hand", label, med[1], REPS);
    0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = match args.get(1).map(|s| s.as_str()) {
        Some("gates") => gates(),
        Some("perf") => match args.get(2).and_then(|s| s.parse::<usize>().ok()) {
            Some(r) if r >= 7 => perf(r, args.get(3).map(|s| s.as_str()).unwrap_or("?")),
            _ => { eprintln!("usage: harness perf <rounds >= 7> [label]"); 2 }
        },
        _ => { eprintln!("usage: harness gates | perf <rounds >= 7> [label]"); 2 }
    };
    std::process::exit(code);
}
