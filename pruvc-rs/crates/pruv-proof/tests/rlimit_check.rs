//! CI-1a/b: verification queries are decided by the z3 resource limit. These pin the two paths of `sys::Solver::check`
//! with an rlimit: a stop at the shared-context first-check cap is retried in the isolated context with the full
//! rlimit, and a query that exhausts the full rlimit ends unknown (no wall clock involved: the timeout is 60 s).
//! The query is the pigeonhole formula for 11 pigeons in 10 holes (unsat). With the pinned z3 (z3-src 503.3.0) it needs more than 1M
//! and less than 5M resource units (measured 2026-11-25); if a z3 upgrade moves it out of that band, re-measure.

use pruv_proof::sys::{Ast, Sat, Solver, Z3};

fn pigeonhole(z: &Z3, holes: usize) -> Vec<Ast> {
    let p: Vec<Vec<Ast>> =
        (0..=holes).map(|i| (0..holes).map(|j| z.bool_const(&format!("p{i}_{j}"))).collect()).collect();
    let mut v: Vec<Ast> = p.iter().map(|row| z.or(row)).collect();
    for j in 0..holes {
        for a in 0..=holes {
            for b in a + 1..=holes {
                v.push(z.or(&[p[a][j].not(), p[b][j].not()]));
            }
        }
    }
    v
}

fn check(rlimit: u32, cap: Option<u32>) -> (Sat, String) {
    let z = Z3::new();
    let mut s = Solver::with_options(&z, 60000, Some(rlimit), None);
    if let Some(c) = cap {
        s.set_shared_first_rlimit(c);
    }
    for a in pigeonhole(&z, 10) {
        s.add(&a);
    }
    let r = s.check();
    (r, s.reason_unknown())
}

#[test]
fn full_rlimit_exhausted_is_unknown() {
    // 1M is below the query's need and below the default cap, so one check runs and stops on the rlimit.
    let (r, why) = check(1_000_000, None);
    assert_eq!(r, Sat::Unknown);
    assert!(!why.contains("timeout"), "stopped by the wall clock, not the rlimit: {why}");
}

#[test]
fn capped_first_check_is_retried_with_full_rlimit() {
    // The shared check gets 1M (stops), the isolated retry gets the full 5M (decides).
    assert_eq!(check(5_000_000, Some(1_000_000)).0, Sat::Unsat);
}

#[test]
fn capped_retry_that_exhausts_full_rlimit_is_unknown() {
    // Both checks stop on the rlimit: the shared one at 500k, the isolated one at the full 1M.
    let (r, why) = check(1_000_000, Some(500_000));
    assert_eq!(r, Sat::Unknown);
    assert!(!why.contains("timeout"), "stopped by the wall clock, not the rlimit: {why}");
}
