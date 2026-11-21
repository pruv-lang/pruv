//! Bounds and old-array facts for overlap-safe copies.

use pruv_core::diagnostic::PruvError;
use pruv_proof::verify_program;

fn verify(src: &str) -> Vec<PruvError> {
    let mut prog = pruv_core::parse(src).expect("parse");
    let tt = pruv_core::check::check(&mut prog).expect("check");
    verify_program(&prog, &tt, None).expect("verify")
}

fn errors(requires: &str) -> Vec<PruvError> {
    verify(&format!(
        "fn put(inout a: [u8], dst: i64, src: i64, n: i64) -> ()\n  requires {}\n{{\n  copy_within(inout a, dst, src, n);\n}}\n",
        requires
    ))
}

#[test]
fn copy_within_proven_bounds_pass() {
    let errs = errors("dst >= 0 && src >= 0 && n >= 0 && dst + n <= len(a) && src + n <= len(a)");
    assert!(errs.is_empty(), "{:?}", errs);
}

fn assert_single_failure(requires: &str, what: &str) {
    let errs = errors(requires);
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert_eq!(errs[0].code, "index_oob");
    assert!(errs[0].message.contains(what), "{}", errs[0].message);
}

#[test]
fn copy_within_dst_negative_fails() {
    assert_single_failure("src >= 0 && n >= 0 && dst + n <= len(a) && src + n <= len(a)", "0 <= dst");
}

#[test]
fn copy_within_src_negative_fails() {
    assert_single_failure("dst >= 0 && n >= 0 && dst + n <= len(a) && src + n <= len(a)", "0 <= src");
}

#[test]
fn copy_within_n_negative_fails() {
    assert_single_failure("dst >= 0 && src >= 0 && dst + n <= len(a) && src + n <= len(a)", "0 <= n");
}

#[test]
fn copy_within_dst_overrun_fails() {
    assert_single_failure("dst >= 0 && src >= 0 && n >= 0 && src + n <= len(a)", "dst + n <= len(a)");
}

#[test]
fn copy_within_src_overrun_fails() {
    assert_single_failure("dst >= 0 && src >= 0 && n >= 0 && dst + n <= len(a)", "src + n <= len(a)");
}

#[test]
fn copy_within_facts_are_provable() {
    let errs = verify(
        "fn put(inout a: [i64], dst: i64, src: i64, n: i64) -> ()\n  requires 0 <= dst && 0 <= src && 0 <= n && dst + n <= len(a) && src + n <= len(a)\n  ensures len(a) == len(old(a))\n  ensures forall k in 0..n: a[dst + k] == old(a)[src + k]\n  ensures forall p in 0..len(a): (p < dst || p >= dst + n) ==> a[p] == old(a)[p]\n{\n  copy_within(inout a, dst, src, n);\n}\n",
    );
    assert!(errs.is_empty(), "{:?}", errs);
}
