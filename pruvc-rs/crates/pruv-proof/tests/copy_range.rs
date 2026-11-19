//! L-copy-range: copy_range — five bounds obligations at each call (0 <= i, 0 <= j, 0 <= n, i + n <= len(dst),
//! j + n <= len(src)), then the content, frame and length facts of the array-theory update.

use pruv_core::diagnostic::PruvError;
use pruv_proof::verify_program;

fn verify(src: &str) -> Vec<PruvError> {
    let mut prog = pruv_core::parse(src).expect("parse");
    let tt = pruv_core::check::check(&mut prog).expect("check");
    verify_program(&prog, &tt, None).expect("verify")
}

fn errors(requires: &str) -> Vec<PruvError> {
    verify(&format!(
        "fn put(inout dst: [u8], i: i64, src: [u8], j: i64, n: i64) -> ()\n  requires {}\n{{\n  copy_range(inout dst, i, src, j, n);\n}}\n",
        requires
    ))
}

#[test]
fn copy_range_proven_bounds_pass() {
    let errs = errors("i >= 0 && j >= 0 && n >= 0 && i + n <= len(dst) && j + n <= len(src)");
    assert!(errs.is_empty(), "{:?}", errs);
}

fn assert_single_failure(requires: &str, what: &str) {
    let errs = errors(requires);
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert_eq!(errs[0].code, "index_oob");
    assert!(errs[0].message.contains(what), "{}", errs[0].message);
}

#[test]
fn copy_range_i_negative_fails() {
    assert_single_failure("j >= 0 && n >= 0 && i + n <= len(dst) && j + n <= len(src)", "0 <= i");
}

#[test]
fn copy_range_j_negative_fails() {
    assert_single_failure("i >= 0 && n >= 0 && i + n <= len(dst) && j + n <= len(src)", "0 <= j");
}

#[test]
fn copy_range_n_negative_fails() {
    assert_single_failure("i >= 0 && j >= 0 && i + n <= len(dst) && j + n <= len(src)", "0 <= n");
}

#[test]
fn copy_range_dst_overrun_fails() {
    assert_single_failure("i >= 0 && j >= 0 && n >= 0 && j + n <= len(src)", "i + n <= len(dst)");
}

#[test]
fn copy_range_src_overrun_fails() {
    assert_single_failure("i >= 0 && j >= 0 && n >= 0 && i + n <= len(dst)", "j + n <= len(src)");
}

#[test]
fn copy_range_facts_are_provable() {
    let errs = verify(
        "fn put(inout dst: [i64], i: i64, src: [i64], j: i64, n: i64) -> ()\n  requires 0 <= i && 0 <= j && 0 <= n && i + n <= len(dst) && j + n <= len(src)\n  ensures len(dst) == len(old(dst))\n  ensures forall k in 0..n: dst[i + k] == src[j + k]\n  ensures forall p in 0..len(dst): (p < i || p >= i + n) ==> dst[p] == old(dst)[p]\n{\n  copy_range(inout dst, i, src, j, n);\n}\n",
    );
    assert!(errs.is_empty(), "{:?}", errs);
}

#[test]
fn copy_range_unsound_facts_fail() {
    // an element outside the range changed
    let errs = verify(
        "fn f(inout dst: [i64], src: [i64]) -> ()\n  requires 4 <= len(dst) && 2 <= len(src)\n{\n  let before: i64 = dst[3];\n  copy_range(inout dst, 0, src, 0, 2);\n  assert dst[3] != before;\n}\n",
    );
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert_eq!(errs[0].code, "assert");
    // content that no copy established
    let errs = verify(
        "fn f(inout dst: [i64], src: [i64]) -> ()\n  requires 2 <= len(dst) && 2 <= len(src) && src[0] != src[1]\n{\n  copy_range(inout dst, 0, src, 0, 1);\n  assert dst[0] == src[1];\n}\n",
    );
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert_eq!(errs[0].code, "assert");
}
