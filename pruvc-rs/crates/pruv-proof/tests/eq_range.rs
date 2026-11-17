//! L3-10: eq_range — five bounds obligations at each call (0 <= i, 0 <= j, 0 <= n, i + n <= len(a), j + n <= len(b)).

use pruv_core::diagnostic::PruvError;
use pruv_proof::verify_program;

fn errors(requires: &str) -> Vec<PruvError> {
    let src = format!(
        "fn same(a: [u8], i: i64, b: [u8], j: i64, n: i64) -> bool\n  requires {}\n{{\n  return eq_range(a, i, b, j, n);\n}}\n",
        requires
    );
    let mut prog = pruv_core::parse(&src).expect("parse");
    let tt = pruv_core::check::check(&mut prog).expect("check");
    verify_program(&prog, &tt, None).expect("verify")
}

#[test]
fn eq_range_proven_bounds_pass() {
    let errs = errors("i >= 0 && j >= 0 && n >= 0 && i + n <= len(a) && j + n <= len(b)");
    assert!(errs.is_empty(), "{:?}", errs);
}

fn assert_single_failure(requires: &str, what: &str) {
    let errs = errors(requires);
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert_eq!(errs[0].code, "index_oob");
    assert!(errs[0].message.contains(what), "{}", errs[0].message);
}

#[test]
fn eq_range_i_negative_fails() {
    assert_single_failure("j >= 0 && n >= 0 && i + n <= len(a) && j + n <= len(b)", "0 <= i");
}

#[test]
fn eq_range_j_negative_fails() {
    assert_single_failure("i >= 0 && n >= 0 && i + n <= len(a) && j + n <= len(b)", "0 <= j");
}

#[test]
fn eq_range_n_negative_fails() {
    assert_single_failure("i >= 0 && j >= 0 && i + n <= len(a) && j + n <= len(b)", "0 <= n");
}

#[test]
fn eq_range_a_overrun_fails() {
    assert_single_failure("i >= 0 && j >= 0 && n >= 0 && j + n <= len(b)", "i + n <= len(a)");
}

#[test]
fn eq_range_b_overrun_fails() {
    assert_single_failure("i >= 0 && j >= 0 && n >= 0 && i + n <= len(a)", "j + n <= len(b)");
}
