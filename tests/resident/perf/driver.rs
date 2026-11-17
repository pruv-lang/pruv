// Resident-state perf host (docs/RESIDENT_STATE_SPEC.md section 8).
//
// This file is NOT compiled on its own. run.py writes a main file that starts with
//     #![allow(dead_code, unused_imports)]
//     #[path = "<generated>.rs"] mod sol;
// followed by this file (the same way tests/run_all.py builds its runtime drivers), so `sol` is the generated
// Rust module (Rust source backend) or the `--rust-bindings` module (LLVM backend). Both expose the same entries:
//     sol::init(&mut [i64], i64)
//     sol::step(&mut [i64], &[i64]) -> i64
//     sol::noop(&mut [i64], &[i64]) -> i64
//     sol::step_copy(&[i64], &[i64]) -> Vec<i64>
// The host only allocates, calls and prints; every decision (index range, header checks) is made in Pruv.
use std::hint::black_box;
use std::time::Instant;

const CALLS: usize = 10_000;
const WARMUP: usize = 1_000;
const WORDS: usize = 4; // record size R, fixed by state.pruv
const HEADER: usize = 4;

// The backend label comes from the environment of the rustc run (run.py sets PERF_BACKEND).
fn backend() -> &'static str {
    option_env!("PERF_BACKEND").unwrap_or("unknown")
}

// Call input for call number j: a spread-out non-negative record index and a delta. Pruv reduces the index mod S.
fn make_call(j: usize, call: &mut [i64; 2]) {
    call[0] = ((j as i64).wrapping_mul(2_654_435_761)) & i64::MAX;
    call[1] = 1 + (j as i64 & 7);
}

fn report(kind: &str, s: usize, elapsed_ns: u128) {
    println!(
        "backend={} S={} kind={} calls={} ns_per_call={:.1}",
        backend(),
        s,
        kind,
        CALLS,
        elapsed_ns as f64 / CALLS as f64
    );
}

fn main() {
    let mut check: i64 = 0;
    for &s in &[2000usize, 20000usize] {
        // The resident buffer: allocated once, initialised by Pruv, never touched by the host afterwards.
        let mut st: Vec<i64> = vec![0; HEADER + WORDS * s];
        sol::init(&mut st[..], s as i64);
        let mut call = [0i64; 2];

        // step: resident in-place update through `inout`.
        for j in 0..WARMUP {
            make_call(j, &mut call);
            check ^= black_box(sol::step(&mut st[..], black_box(&call[..])));
        }
        let t = Instant::now();
        for j in 0..CALLS {
            make_call(j, &mut call);
            check ^= black_box(sol::step(&mut st[..], black_box(&call[..])));
        }
        report("step", s, t.elapsed().as_nanos());

        // noop: fixed cost of one boundary call.
        for j in 0..WARMUP {
            make_call(j, &mut call);
            check ^= black_box(sol::noop(&mut st[..], black_box(&call[..])));
        }
        let t = Instant::now();
        for j in 0..CALLS {
            make_call(j, &mut call);
            check ^= black_box(sol::noop(&mut st[..], black_box(&call[..])));
        }
        report("noop", s, t.elapsed().as_nanos());

        // copy: the pre-batch shape. The host replaces its Vec with the returned fresh copy.
        for j in 0..WARMUP {
            make_call(j, &mut call);
            st = black_box(sol::step_copy(&st[..], black_box(&call[..])));
        }
        let t = Instant::now();
        for j in 0..CALLS {
            make_call(j, &mut call);
            st = black_box(sol::step_copy(&st[..], black_box(&call[..])));
        }
        report("copy", s, t.elapsed().as_nanos());
        check ^= black_box(st.len() as i64);
    }
    // Keeps every result observable; not part of the measurement lines.
    eprintln!("check={}", check);
}
