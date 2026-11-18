// P09 panic-injection host (tests/l3/abi/check_raw_entries.py). check_raw_entries.py copies this file to main.rs next
// to the generated gen.rs (Rust backend, manifest raw_entries = "crate", built with --no-verify), so `sol` below is the
// generated module. The host keeps the resident state in sol::pruv_raw::PruvStore<Vec<i64>> and calls the pub(crate)
// entries only inside `with`.
#![allow(dead_code, unused_imports)]
#[path = "gen.rs"]
mod sol;

use sol::pruv_raw::PruvStore;
use std::panic::{catch_unwind, AssertUnwindSafe};

fn expect(bad: &mut u32, what: &str, c: bool) {
    if !c {
        *bad += 1;
        eprintln!("FAIL {}", what);
    }
}

// One poisoning scenario: a normal call, then `body` panics inside `with`, the panic is caught, and the store must
// stay poisoned: later `with` calls (mutating or observing) are refused without running their closure, and dropping
// the poisoned store is fine.
fn scenario(name: &str, bad: &mut u32, body: impl FnOnce(&mut Vec<i64>)) {
    let mut st = PruvStore::new(vec![0i64; 4]);
    let first = st.with(|v| {
        sol::put(&mut v[..], 1);
        sol::get(&v[..], 0)
    });
    expect(bad, &format!("{}: first call", name), matches!(first, Ok(1)) && !st.is_poisoned());
    let caught = catch_unwind(AssertUnwindSafe(|| st.with(|v| body(v))));
    expect(bad, &format!("{}: panic caught", name), caught.is_err());
    expect(bad, &format!("{}: is_poisoned", name), st.is_poisoned());
    let mut ran = false;
    let again = st.with(|_| {
        ran = true;
    });
    expect(bad, &format!("{}: later with refused", name), again.is_err() && !ran);
    let mut ran_obs = false;
    let obs = st.with(|v| {
        ran_obs = true;
        v[0]
    });
    expect(bad, &format!("{}: observation refused", name), obs.is_err() && !ran_obs);
    expect(bad, &format!("{}: still poisoned", name), st.is_poisoned());
    drop(st);
    println!("{}: poisoned, refused, dropped", name);
}

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let mut bad = 0u32;

    // partial write: the entry writes st[0] and then indexes st[100] of a 4-word state
    scenario("partial_write", &mut bad, |v| {
        sol::put_far(&mut v[..], 100);
    });

    // allocation failure: capacity overflow is a panic and unwinds. (A real out-of-memory aborts through
    // handle_alloc_error and never unwinds, so it cannot be caught or tested here.)
    scenario("alloc_failure", &mut bad, |v| {
        sol::put(&mut v[..], 5);
        let big = Vec::<i64>::with_capacity(usize::MAX);
        std::hint::black_box(big.capacity());
    });

    // host panic after the entry wrote
    scenario("host_panic", &mut bad, |v| {
        sol::put(&mut v[..], 6);
        panic!("host panic");
    });

    // normal clear: a fresh store, several calls, never poisoned
    let mut st = PruvStore::new(vec![0i64; 4]);
    let mut clear = true;
    for k in 1..=3i64 {
        let r = st.with(|v| {
            sol::put(&mut v[..], k * 10);
            sol::get(&v[..], 0)
        });
        clear &= matches!(r, Ok(x) if x == k * 10) && !st.is_poisoned();
    }
    expect(&mut bad, "normal: clear", clear);
    println!("normal: ok");

    if bad != 0 {
        std::process::exit(1);
    }
    println!("summary: 3 poisoned, 1 clear");
}
