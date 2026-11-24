//! Pruv -> LLVM IR text generator (a port of llvmgen.py lines 1-1327 plus gen_llvm).
//! Goal: for the same program and mode, byte-identical to the first return value of Python's gen_llvm.
//! Rule: mutable state (temp numbers, label numbers, scopes, temp-free lists, contract environment) is changed in the same order as in Python.
//! Python's None is imitated by the string "None" (the default of V's a/b).

use crate::storage::{plan_storage, StoragePlan, elem_size, alignment};
use crate::abi::{abi_of, AbiTarget};
use crate::ast::*;
use crate::typeinfo::{is_array_name, is_user_type, uses_array_type, TypeTable};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub type R<T> = Result<T, String>;

// ---------- G6 A3 (docs/G6_SPEC.md section 13): collections and trusted calls through the shim ----------

pub const G6_UNSUPPORTED_MSG: &str = "G6 foreign calls and collections are not implemented by the llvm codegen layer yet";
pub const G6_UNSUPPORTED_FIX: &str = "avoid trusted rust fn, Vec, HashMap, String, take and move for now";
pub const G6_BRIDGE_DECL: &str = "@pruv_g6_bridge_v1 = external global i8\n";

/// P10H: the prefix of every program-specific non-internal global symbol of the LLVM build (`pruv_abi_*`,
/// `pruv_g6_v1_*`, `pruv_g6_bridge_v1`, the C adapter): `<pkg>__` for a program with a package name, so two programs
/// link into one binary. A program without a package name keeps the unprefixed names (it cannot be linked together
/// with another program that defines the same symbols). The program-independent runtime is never prefixed.
pub fn sym_prefix(pkg: Option<&str>) -> String {
    match pkg {
        Some(p) if !p.is_empty() => format!("{}__", p),
        _ => String::new(),
    }
}

/// P10H: puts `pre` in front of the name of every `define .. @pruv_abi_*` line of a wrapper text produced by
/// crate::abi (first `@pruv_abi_` of a `define` line only; the callee names of the bodies are internal).
fn prefix_abi_defs(text: &str, pre: &str) -> String {
    if pre.is_empty() {
        return text.to_string();
    }
    let to = format!(" @{}pruv_abi_", pre);
    text.split('\n')
        .map(|l| if l.starts_with("define ") { l.replacen(" @pruv_abi_", &to, 1) } else { l.to_string() })
        .collect::<Vec<String>>()
        .join("\n")
}
/// A PruvError (codegen/unsupported_type) raised in the middle of a function body travels through `R<T>` as a string
/// with this prefix and `line:col`; `gen_llvm_concrete_ex` turns it back into the diagnostic.
const G6_DIAG_TAG: &str = "\u{0}g6diag:";

fn g6_diag_err(line: i64, col: i64) -> String {
    format!("{}{}:{}", G6_DIAG_TAG, line, col)
}

/// G9 A3: a PruvError (codegen/unsupported_type, Python `FnGen.g9_unsupported`) raised in the middle of a function body
/// travels like the G6 one: this prefix, then `line:col:what`; `gen_llvm_concrete_ex` turns it back into the diagnostic.
const G9_DIAG_TAG: &str = "\u{0}g9diag:";
const G9_COST_DIAG_TAG: &str = "\u{0}g9cdiag:";

/// P17a: an eq_range the LLVM backend does not lower (Python FnGen ev_call raises codegen/unsupported_type):
/// this prefix, then `line:col:what` (the operand or layout that is not lowered).
const EQR_DIAG_TAG: &str = "\u{0}eqrdiag:";

/// L-copy-range: a copy_range the LLVM backend does not lower; same `line:col:what` payload as EQR_DIAG_TAG.
const CPR_DIAG_TAG: &str = "\u{0}cprdiag:";

/// P17a: a dynamic-length eq_range below this many elements uses the inline loop, else memcmp (Python EQR_LOOP_MAX).
const EQR_LOOP_MAX: i64 = 4;

fn g9_diag_err(line: i64, col: i64, what: &str) -> String {
    format!("{}{}:{}:{}", G9_DIAG_TAG, line, col, what)
}

const E2_DIAG_TAG: &str = "\u{0}e2diag:";

pub(crate) fn is_owned_result(t: &str) -> bool {
    t.starts_with("result<") && g_error(t) == "string"
}

fn e2_diag(line: i64, col: i64, message: &str) -> String {
    format!("{E2_DIAG_TAG}{line}:{col}:{message}")
}

/// Pruv's checked recursive vectors use an inline LLVM vector header.
pub(crate) fn native_vec_elem(t: &str) -> Option<&str> {
    let elem = t.strip_prefix("vec<")?.strip_suffix('>')?;
    (crate::emit_common::is_user_key(elem) && !elem.contains('<')).then_some(elem)
}

pub(crate) fn recursive_vec_elements(prog: &Program) -> BTreeSet<String> {
    let mut elems = BTreeSet::new();
    if let Some(meta) = &prog.g9 {
        for group in &meta.type_sccs {
            for decl in prog.types.iter().filter(|d| group.iter().any(|n| n == d.name())) {
                for field in decl.all_fields() {
                    if let Some(elem) = native_vec_elem(&field.ty) {
                        if group.iter().any(|n| n == elem) {
                            elems.insert(elem.to_string());
                        }
                    }
                }
            }
        }
    }
    elems
}

/// An opaque collection/File handle type key; never a bound parameter.
pub fn is_collection(t: &str) -> bool {
    if t.starts_with('\u{1}') || native_vec_elem(t).is_some() {
        return false;
    }
    matches!(crate::emit_common::key_name(t), "vec" | "hashmap" | "string" | "file" | "process" | "process_args" | "socket" | "listener" | "loop")
}

/// G6: a collection type or one anywhere inside its type arguments.
pub fn contains_collection(t: &str) -> bool {
    match crate::gtype::GType::try_from_key(t) {
        Ok(g) => g.contains_collection(),
        Err(_) => false,
    }
}

pub const SIGNED_MIN128: i128 = i128::MIN;
/// P2: bit builtin -> LLVM instruction (Python BIT_LLVM); later batches add entries.
const BIT_LLVM: [(&str, &str); 7] = [
    ("band", "and"),
    ("bor", "or"),
    ("bxor", "xor"),
    ("wrapping_shl", "shl"),
    ("ushr", "lshr"),
    ("wrapping_add", "add"),
    ("wrapping_mul", "mul"),
];

fn bit_operator_llvm(op: &str) -> Option<&'static str> {
    match op {
        "&" => Some("and"),
        "|" => Some("or"),
        "^" => Some("xor"),
        "<<" => Some("shl"),
        ">>" => Some("ashr"),
        _ => None,
    }
}

fn bit_llvm(n: &str) -> Option<&'static str> {
    BIT_LLVM.iter().find(|(k, _)| *k == n).map(|(_, ins)| *ins)
}
pub const OPT: &str = "{ i1, i64 }";
/// Option/Result stored inside a contract aggregate
pub const COPT: &str = "{ i1, i128 }";
pub const ARR: &str = "{ ptr, i64, i64 }"; // P3: owned buffer inside the module: { data, len, cap }; cap = allocated element count
/// R-6g9-1: a borrowed view result `{ data, len }`; it owns nothing and is never freed
pub const VIEW: &str = "{ ptr, i64 }";

/// R-6g9-1: the function returns a `ref [u8]` / `ref str` view of its `from` sources
pub fn view_ret(f: &Function) -> bool {
    f.borrowed_result && is_arr_name(&f.ret)
}

// P3: pruv_alloc / pruv_copy (and pruv_alloc_u8 / pruv_copy_v2 in DECLS_U8) are inline module copies of the
// runtime functions of the same name: len <= 0 gives NULL, a len above the runtime's bound panics with the same
// message, and the block comes from pruv_raw_alloc_v3 / pruv_raw_alloc_zeroed_v3 (runtime.c), so the constant
// layout and length checks fold away and only the allocator call is left.
// The allocator entry points (and pruv_free / pruv_free_v2) touch only allocator state the module cannot see, plus
// the freed block itself, so they carry memory(inaccessiblemem / argmem) and a call no longer clobbers the caller's
// loaded values. No willreturn: an allocation failure ends the process.
pub const DECLS: &str = concat!(
    "declare noalias nonnull ptr @pruv_raw_alloc_v3(i64, i64, i64) nounwind memory(inaccessiblemem: readwrite)\n",
    "declare noalias nonnull ptr @pruv_raw_alloc_zeroed_v3(i64, i64, i64) nounwind memory(inaccessiblemem: readwrite)\n",
    "@pruv_alloc_failed_msg = private unnamed_addr constant [18 x i8] c\"allocation failed\\00\"\n",
    "define internal noalias ptr @pruv_alloc(i64 %n, i64 %v) alwaysinline nounwind {\n",
    "entry:\n",
    "  %e = icmp sle i64 %n, 0\n",
    "  br i1 %e, label %none, label %chk\n",
    "none:\n",
    "  ret ptr null\n",
    "chk:\n",
    "  %big = icmp ugt i64 %n, 1152921504606846975\n",
    "  br i1 %big, label %fail, label %ok\n",
    "fail:\n",
    "  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)\n",
    "  unreachable\n",
    "ok:\n",
    "  %z = icmp eq i64 %v, 0\n",
    "  br i1 %z, label %zero, label %fill\n",
    "zero:\n",
    "  %p = call ptr @pruv_raw_alloc_zeroed_v3(i64 %n, i64 8, i64 8)\n",
    "  ret ptr %p\n",
    "fill:\n",
    "  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 8, i64 8)\n",
    "  br label %loop\n",
    "loop:\n",
    "  %i = phi i64 [ 0, %fill ], [ %i1, %loop ]\n",
    "  %g = getelementptr inbounds i64, ptr %q, i64 %i\n",
    "  store i64 %v, ptr %g\n",
    "  %i1 = add nuw nsw i64 %i, 1\n",
    "  %d = icmp eq i64 %i1, %n\n",
    "  br i1 %d, label %done, label %loop\n",
    "done:\n",
    "  ret ptr %q\n",
    "}\n",
    "define internal noalias ptr @pruv_copy(ptr nocapture readonly %s, i64 %n) alwaysinline nounwind {\n",
    "entry:\n",
    "  %e = icmp sle i64 %n, 0\n",
    "  br i1 %e, label %none, label %chk\n",
    "none:\n",
    "  ret ptr null\n",
    "chk:\n",
    "  %big = icmp ugt i64 %n, 1152921504606846975\n",
    "  br i1 %big, label %fail, label %ok\n",
    "fail:\n",
    "  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)\n",
    "  unreachable\n",
    "ok:\n",
    "  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 8, i64 8)\n",
    "  %b = shl nuw nsw i64 %n, 3\n",
    "  call void @llvm.memcpy.p0.p0.i64(ptr %q, ptr %s, i64 %b, i1 false)\n",
    "  ret ptr %q\n",
    "}\n",
    "declare void @llvm.memcpy.p0.p0.i64(ptr noalias nocapture writeonly, ptr noalias nocapture readonly, i64, i1 immarg)\n",
    "declare void @llvm.memset.p0.i64(ptr nocapture writeonly, i8, i64, i1)\n",
    "declare void @pruv_free(ptr nocapture, i64) nounwind memory(argmem: readwrite, inaccessiblemem: readwrite)\n",
    "declare void @pruv_print_i64(i64) nounwind\n",
    "declare void @pruv_print_bool(i64) nounwind\n",
    "declare void @pruv_panic_oob(i64, i64) noreturn cold nounwind\n",
    "declare void @pruv_panic_arith(ptr) noreturn cold nounwind\n",
    "declare void @pruv_panic_requires(ptr, ptr) noreturn cold nounwind\n",
    "declare void @pruv_panic_undefined(ptr) noreturn cold nounwind\n",
    "declare { i64, i1 } @llvm.sadd.with.overflow.i64(i64, i64)\n",
    "declare { i64, i1 } @llvm.ssub.with.overflow.i64(i64, i64)\n",
    "declare { i64, i1 } @llvm.smul.with.overflow.i64(i64, i64)\n",
    "declare { i128, i1 } @llvm.sadd.with.overflow.i128(i128, i128)\n",
    "declare { i128, i1 } @llvm.ssub.with.overflow.i128(i128, i128)\n",
    "declare { i128, i1 } @llvm.smul.with.overflow.i128(i128, i128)\n",
    "define internal void @pruv_free_nn(ptr %p, i64 %n) alwaysinline nounwind {\n",
    "entry:\n",
    "  %z = icmp eq ptr %p, null\n",
    "  br i1 %z, label %done, label %do\n",
    "do:\n",
    "  call void @pruv_free(ptr %p, i64 %n)\n",
    "  br label %done\n",
    "done:\n",
    "  ret void\n",
    "}\n",
);

/// Declarations appended right after DECLS only for `[u8]` programs (G2 A4 §2.9).
pub const DECLS_U8: &str = concat!(
    "define internal noalias ptr @pruv_alloc_u8(i64 %n, i64 %v) alwaysinline nounwind {\n",
    "entry:\n",
    "  %e = icmp sle i64 %n, 0\n",
    "  br i1 %e, label %none, label %chk\n",
    "none:\n",
    "  ret ptr null\n",
    "chk:\n",
    "  %big = icmp ugt i64 %n, 1099511627776\n",
    "  br i1 %big, label %fail, label %ok\n",
    "fail:\n",
    "  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)\n",
    "  unreachable\n",
    "ok:\n",
    "  %z = icmp eq i64 %v, 0\n",
    "  br i1 %z, label %zero, label %fill\n",
    "zero:\n",
    "  %p = call ptr @pruv_raw_alloc_zeroed_v3(i64 %n, i64 1, i64 1)\n",
    "  ret ptr %p\n",
    "fill:\n",
    "  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 1, i64 1)\n",
    "  %b = trunc i64 %v to i8\n",
    "  call void @llvm.memset.p0.i64(ptr %q, i8 %b, i64 %n, i1 false)\n",
    "  ret ptr %q\n",
    "}\n",
    "define internal noalias ptr @pruv_copy_v2(ptr nocapture readonly %s, i64 %n, i64 %sz, i64 %al) alwaysinline nounwind {\n",
    "entry:\n",
    "  %e = icmp sle i64 %n, 0\n",
    "  br i1 %e, label %none, label %chk\n",
    "none:\n",
    "  ret ptr null\n",
    "chk:\n",
    "  %big = icmp ugt i64 %n, 1099511627776\n",
    "  br i1 %big, label %fail, label %ok\n",
    "fail:\n",
    "  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)\n",
    "  unreachable\n",
    "ok:\n",
    "  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 %sz, i64 %al)\n",
    "  %b = mul nuw nsw i64 %n, %sz\n",
    "  call void @llvm.memcpy.p0.p0.i64(ptr %q, ptr %s, i64 %b, i1 false)\n",
    "  ret ptr %q\n",
    "}\n",
    "declare void @pruv_free_v2(ptr nocapture, i64, i64, i64) nounwind memory(argmem: readwrite, inaccessiblemem: readwrite)\n",
    "declare void @pruv_panic_elem(i64, ptr) noreturn cold nounwind\n",
    "define internal void @pruv_free_v2_nn(ptr %p, i64 %n, i64 %s, i64 %a) alwaysinline nounwind {\n",
    "entry:\n",
    "  %z = icmp eq ptr %p, null\n",
    "  br i1 %z, label %done, label %do\n",
    "do:\n",
    "  call void @pruv_free_v2(ptr %p, i64 %n, i64 %s, i64 %a)\n",
    "  br label %done\n",
    "done:\n",
    "  ret void\n",
    "}\n",
);

/// P3: only emitted when a tail-filled alloc is lowered (FnGen::tail_fill): `alloc(n, 0)` whose first k elements the
/// next loop overwrites zeroes only elements k..n (k clamped to 0..n); a large block still comes from the zeroed
/// allocator, whose fresh pages are already zero.
pub const DECLS_TAIL: &str = concat!(
    "define internal noalias ptr @pruv_alloc_tail(i64 %n, i64 %k) alwaysinline nounwind {\n",
    "entry:\n",
    "  %e = icmp sle i64 %n, 0\n",
    "  br i1 %e, label %none, label %chk\n",
    "none:\n",
    "  ret ptr null\n",
    "chk:\n",
    "  %big = icmp ugt i64 %n, 1152921504606846975\n",
    "  br i1 %big, label %fail, label %ok\n",
    "fail:\n",
    "  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)\n",
    "  unreachable\n",
    "ok:\n",
    "  %large = icmp ugt i64 %n, 16384\n",
    "  br i1 %large, label %zero, label %part\n",
    "zero:\n",
    "  %p = call ptr @pruv_raw_alloc_zeroed_v3(i64 %n, i64 8, i64 8)\n",
    "  ret ptr %p\n",
    "part:\n",
    "  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 8, i64 8)\n",
    "  %k0 = call i64 @llvm.smax.i64(i64 %k, i64 0)\n",
    "  %k1 = call i64 @llvm.smin.i64(i64 %k0, i64 %n)\n",
    "  %g = getelementptr inbounds i64, ptr %q, i64 %k1\n",
    "  %r = sub nuw nsw i64 %n, %k1\n",
    "  %b = shl nuw nsw i64 %r, 3\n",
    "  call void @llvm.memset.p0.i64(ptr %g, i8 0, i64 %b, i1 false)\n",
    "  ret ptr %q\n",
    "}\n",
);
pub const DECLS_TAIL_U8: &str = concat!(
    "define internal noalias ptr @pruv_alloc_tail_u8(i64 %n, i64 %k) alwaysinline nounwind {\n",
    "entry:\n",
    "  %e = icmp sle i64 %n, 0\n",
    "  br i1 %e, label %none, label %chk\n",
    "none:\n",
    "  ret ptr null\n",
    "chk:\n",
    "  %big = icmp ugt i64 %n, 1099511627776\n",
    "  br i1 %big, label %fail, label %ok\n",
    "fail:\n",
    "  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)\n",
    "  unreachable\n",
    "ok:\n",
    "  %large = icmp ugt i64 %n, 131072\n",
    "  br i1 %large, label %zero, label %part\n",
    "zero:\n",
    "  %p = call ptr @pruv_raw_alloc_zeroed_v3(i64 %n, i64 1, i64 1)\n",
    "  ret ptr %p\n",
    "part:\n",
    "  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 1, i64 1)\n",
    "  %k0 = call i64 @llvm.smax.i64(i64 %k, i64 0)\n",
    "  %k1 = call i64 @llvm.smin.i64(i64 %k0, i64 %n)\n",
    "  %g = getelementptr inbounds i8, ptr %q, i64 %k1\n",
    "  %r = sub nuw nsw i64 %n, %k1\n",
    "  call void @llvm.memset.p0.i64(ptr %g, i8 0, i64 %r, i1 false)\n",
    "  ret ptr %q\n",
    "}\n",
);
pub const DECLS_MINMAX: &str = "declare i64 @llvm.smax.i64(i64, i64)\ndeclare i64 @llvm.smin.i64(i64, i64)\n";
/// P05G: only emitted when resize is lowered.  resize(a, n, v) with m = len(a) <= n: one uninitialized block of n
/// elements, one memcpy of the m-element prefix, then the n - m tail is set to v (a memset when v is 0 or the
/// elements are bytes).  Every element is written before the block is returned.  n <= 0 gives NULL like pruv_alloc.
pub const DECLS_RESIZE: &str = concat!(
    "define internal noalias ptr @pruv_resize(ptr nocapture readonly %s, i64 %m, i64 %n, i64 %v) alwaysinline nounwind {\n",
    "entry:\n",
    "  %e = icmp sle i64 %n, 0\n",
    "  br i1 %e, label %none, label %chk\n",
    "none:\n",
    "  ret ptr null\n",
    "chk:\n",
    "  %big = icmp ugt i64 %n, 1152921504606846975\n",
    "  br i1 %big, label %fail, label %ok\n",
    "fail:\n",
    "  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)\n",
    "  unreachable\n",
    "ok:\n",
    "  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 8, i64 8)\n",
    "  %b = shl nuw nsw i64 %m, 3\n",
    "  call void @llvm.memcpy.p0.p0.i64(ptr %q, ptr %s, i64 %b, i1 false)\n",
    "  %t = getelementptr inbounds i64, ptr %q, i64 %m\n",
    "  %r = sub nuw nsw i64 %n, %m\n",
    "  %z = icmp eq i64 %v, 0\n",
    "  br i1 %z, label %zero, label %fill\n",
    "zero:\n",
    "  %rb = shl nuw nsw i64 %r, 3\n",
    "  call void @llvm.memset.p0.i64(ptr %t, i8 0, i64 %rb, i1 false)\n",
    "  ret ptr %q\n",
    "fill:\n",
    "  %w = icmp eq i64 %r, 0\n",
    "  br i1 %w, label %done, label %loop\n",
    "loop:\n",
    "  %i = phi i64 [ 0, %fill ], [ %i1, %loop ]\n",
    "  %g = getelementptr inbounds i64, ptr %t, i64 %i\n",
    "  store i64 %v, ptr %g\n",
    "  %i1 = add nuw nsw i64 %i, 1\n",
    "  %d = icmp eq i64 %i1, %r\n",
    "  br i1 %d, label %done, label %loop\n",
    "done:\n",
    "  ret ptr %q\n",
    "}\n",
);
pub const DECLS_RESIZE_U8: &str = concat!(
    "define internal noalias ptr @pruv_resize_u8(ptr nocapture readonly %s, i64 %m, i64 %n, i64 %v) alwaysinline nounwind {\n",
    "entry:\n",
    "  %e = icmp sle i64 %n, 0\n",
    "  br i1 %e, label %none, label %chk\n",
    "none:\n",
    "  ret ptr null\n",
    "chk:\n",
    "  %big = icmp ugt i64 %n, 1099511627776\n",
    "  br i1 %big, label %fail, label %ok\n",
    "fail:\n",
    "  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)\n",
    "  unreachable\n",
    "ok:\n",
    "  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 1, i64 1)\n",
    "  call void @llvm.memcpy.p0.p0.i64(ptr %q, ptr %s, i64 %m, i1 false)\n",
    "  %t = getelementptr inbounds i8, ptr %q, i64 %m\n",
    "  %r = sub nuw nsw i64 %n, %m\n",
    "  %b = trunc i64 %v to i8\n",
    "  call void @llvm.memset.p0.i64(ptr %t, i8 %b, i64 %r, i1 false)\n",
    "  ret ptr %q\n",
    "}\n",
);
/// LG32 (G-DB52-4): only emitted when `x = resize(x, n, v)` is lowered in place.  The block s (allocated count c, live
/// count m <= n) grows through pruv_regrow_v2 (realloc, so the old block is never live next to the new one) when n > c,
/// else it is kept; then the tail [m, n) is set to v exactly like pruv_resize / pruv_resize_u8.  n <= m (a shrink, which
/// R-6g9-1's scratch resize of an inout view source allows, or an empty array) returns s unchanged.
pub const DECLS_REGROW: &str = concat!(
    "define internal ptr @pruv_regrow(ptr %s, i64 %c, i64 %m, i64 %n, i64 %v) alwaysinline nounwind {\n",
    "entry:\n",
    "  %e = icmp sle i64 %n, %m\n",
    "  br i1 %e, label %none, label %chk\n",
    "none:\n",
    "  ret ptr %s\n",
    "chk:\n",
    "  %big = icmp ugt i64 %n, 1152921504606846975\n",
    "  br i1 %big, label %fail, label %ok\n",
    "fail:\n",
    "  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)\n",
    "  unreachable\n",
    "ok:\n",
    "  %gr = icmp sgt i64 %n, %c\n",
    "  br i1 %gr, label %grow, label %have\n",
    "grow:\n",
    "  %p = call ptr @pruv_regrow_v2(ptr %s, i64 %c, i64 %n, i64 8, i64 8)\n",
    "  br label %have\n",
    "have:\n",
    "  %q = phi ptr [ %s, %ok ], [ %p, %grow ]\n",
    "  %t = getelementptr inbounds i64, ptr %q, i64 %m\n",
    "  %r = sub nuw nsw i64 %n, %m\n",
    "  %z = icmp eq i64 %v, 0\n",
    "  br i1 %z, label %zero, label %fill\n",
    "zero:\n",
    "  %rb = shl nuw nsw i64 %r, 3\n",
    "  call void @llvm.memset.p0.i64(ptr %t, i8 0, i64 %rb, i1 false)\n",
    "  ret ptr %q\n",
    "fill:\n",
    "  %w = icmp eq i64 %r, 0\n",
    "  br i1 %w, label %done, label %loop\n",
    "loop:\n",
    "  %i = phi i64 [ 0, %fill ], [ %i1, %loop ]\n",
    "  %g = getelementptr inbounds i64, ptr %t, i64 %i\n",
    "  store i64 %v, ptr %g\n",
    "  %i1 = add nuw nsw i64 %i, 1\n",
    "  %d = icmp eq i64 %i1, %r\n",
    "  br i1 %d, label %done, label %loop\n",
    "done:\n",
    "  ret ptr %q\n",
    "}\n",
);
pub const DECLS_REGROW_U8: &str = concat!(
    "define internal ptr @pruv_regrow_u8(ptr %s, i64 %c, i64 %m, i64 %n, i64 %v) alwaysinline nounwind {\n",
    "entry:\n",
    "  %e = icmp sle i64 %n, %m\n",
    "  br i1 %e, label %none, label %chk\n",
    "none:\n",
    "  ret ptr %s\n",
    "chk:\n",
    "  %big = icmp ugt i64 %n, 1099511627776\n",
    "  br i1 %big, label %fail, label %ok\n",
    "fail:\n",
    "  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)\n",
    "  unreachable\n",
    "ok:\n",
    "  %gr = icmp sgt i64 %n, %c\n",
    "  br i1 %gr, label %grow, label %have\n",
    "grow:\n",
    "  %p = call ptr @pruv_regrow_v2(ptr %s, i64 %c, i64 %n, i64 1, i64 1)\n",
    "  br label %have\n",
    "have:\n",
    "  %q = phi ptr [ %s, %ok ], [ %p, %grow ]\n",
    "  %t = getelementptr inbounds i8, ptr %q, i64 %m\n",
    "  %r = sub nuw nsw i64 %n, %m\n",
    "  %b = trunc i64 %v to i8\n",
    "  call void @llvm.memset.p0.i64(ptr %t, i8 %b, i64 %r, i1 false)\n",
    "  ret ptr %q\n",
    "}\n",
);
// LG34 (G-LG32-3): the regrow of a narrow (u8/i8/u16/i16/u32/i32) storage layout; size/align are the element's
// constant byte width (1, 2 or 4) so the branches fold after inlining.  Same growth as pruv_regrow, same tail fill as
// pruv_resize_narrow (a memset for 0 or byte elements, else one store loop).
pub const DECLS_REGROW_NARROW: &str = concat!(
    "define internal ptr @pruv_regrow_narrow(ptr %s, i64 %c, i64 %m, i64 %n, i64 %v, i64 %size, i64 %align) alwaysinline nounwind {\n",
    "entry:\n",
    "  %e = icmp sle i64 %n, %m\n",
    "  br i1 %e, label %none, label %chk\n",
    "none:\n",
    "  ret ptr %s\n",
    "chk:\n",
    "  %big = icmp ugt i64 %n, 1099511627776\n",
    "  br i1 %big, label %fail, label %ok\n",
    "fail:\n",
    "  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)\n",
    "  unreachable\n",
    "ok:\n",
    "  %gr = icmp sgt i64 %n, %c\n",
    "  br i1 %gr, label %grow, label %have\n",
    "grow:\n",
    "  %p = call ptr @pruv_regrow_v2(ptr %s, i64 %c, i64 %n, i64 %size, i64 %align)\n",
    "  br label %have\n",
    "have:\n",
    "  %q = phi ptr [ %s, %ok ], [ %p, %grow ]\n",
    "  %b = mul nuw nsw i64 %m, %size\n",
    "  %t = getelementptr inbounds i8, ptr %q, i64 %b\n",
    "  %r = sub nuw nsw i64 %n, %m\n",
    "  %z = icmp eq i64 %v, 0\n",
    "  %one = icmp eq i64 %size, 1\n",
    "  %set = or i1 %z, %one\n",
    "  br i1 %set, label %bytes, label %wide\n",
    "bytes:\n",
    "  %rb = mul nuw nsw i64 %r, %size\n",
    "  %vb = trunc i64 %v to i8\n",
    "  call void @llvm.memset.p0.i64(ptr %t, i8 %vb, i64 %rb, i1 false)\n",
    "  ret ptr %q\n",
    "wide:\n",
    "  %w = icmp eq i64 %r, 0\n",
    "  br i1 %w, label %done, label %pick\n",
    "pick:\n",
    "  %two = icmp eq i64 %size, 2\n",
    "  %v16 = trunc i64 %v to i16\n",
    "  %v32 = trunc i64 %v to i32\n",
    "  br i1 %two, label %loop16, label %loop32\n",
    "loop16:\n",
    "  %i = phi i64 [ 0, %pick ], [ %i1, %loop16 ]\n",
    "  %g = getelementptr inbounds i16, ptr %t, i64 %i\n",
    "  store i16 %v16, ptr %g\n",
    "  %i1 = add nuw nsw i64 %i, 1\n",
    "  %d = icmp eq i64 %i1, %r\n",
    "  br i1 %d, label %done, label %loop16\n",
    "loop32:\n",
    "  %j = phi i64 [ 0, %pick ], [ %j1, %loop32 ]\n",
    "  %h = getelementptr inbounds i32, ptr %t, i64 %j\n",
    "  store i32 %v32, ptr %h\n",
    "  %j1 = add nuw nsw i64 %j, 1\n",
    "  %k = icmp eq i64 %j1, %r\n",
    "  br i1 %k, label %done, label %loop32\n",
    "done:\n",
    "  ret ptr %q\n",
    "}\n",
);
pub const DECL_REGROW_V2: &str = "declare ptr @pruv_regrow_v2(ptr, i64, i64, i64, i64) nounwind\n";
/// P05G: resize of a narrow (u8/i8/u16/i16/u32/i32) storage layout, size/align are the element's constant byte width
/// (1, 2 or 4) so the branches fold after inlining.  Same shape as pruv_resize: one uninitialized block, one memcpy
/// of the prefix, the tail set once (a memset for 0 or byte elements, else one store loop).  n <= 2^40 is proved
/// (narrow layouts exist only in verified builds); the check mirrors pruv_resize_u8.
pub const DECLS_RESIZE_NARROW: &str = concat!(
    "define internal noalias ptr @pruv_resize_narrow(ptr nocapture readonly %s, i64 %m, i64 %n, i64 %v, i64 %size, i64 %align) alwaysinline nounwind {\n",
    "entry:\n",
    "  %e = icmp sle i64 %n, 0\n",
    "  br i1 %e, label %none, label %chk\n",
    "none:\n",
    "  ret ptr null\n",
    "chk:\n",
    "  %big = icmp ugt i64 %n, 1099511627776\n",
    "  br i1 %big, label %fail, label %ok\n",
    "fail:\n",
    "  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)\n",
    "  unreachable\n",
    "ok:\n",
    "  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 %size, i64 %align)\n",
    "  %b = mul nuw nsw i64 %m, %size\n",
    "  call void @llvm.memcpy.p0.p0.i64(ptr %q, ptr %s, i64 %b, i1 false)\n",
    "  %t = getelementptr inbounds i8, ptr %q, i64 %b\n",
    "  %r = sub nuw nsw i64 %n, %m\n",
    "  %z = icmp eq i64 %v, 0\n",
    "  %one = icmp eq i64 %size, 1\n",
    "  %set = or i1 %z, %one\n",
    "  br i1 %set, label %bytes, label %wide\n",
    "bytes:\n",
    "  %rb = mul nuw nsw i64 %r, %size\n",
    "  %vb = trunc i64 %v to i8\n",
    "  call void @llvm.memset.p0.i64(ptr %t, i8 %vb, i64 %rb, i1 false)\n",
    "  ret ptr %q\n",
    "wide:\n",
    "  %w = icmp eq i64 %r, 0\n",
    "  br i1 %w, label %done, label %pick\n",
    "pick:\n",
    "  %two = icmp eq i64 %size, 2\n",
    "  %v16 = trunc i64 %v to i16\n",
    "  %v32 = trunc i64 %v to i32\n",
    "  br i1 %two, label %loop16, label %loop32\n",
    "loop16:\n",
    "  %i = phi i64 [ 0, %pick ], [ %i1, %loop16 ]\n",
    "  %g = getelementptr inbounds i16, ptr %t, i64 %i\n",
    "  store i16 %v16, ptr %g\n",
    "  %i1 = add nuw nsw i64 %i, 1\n",
    "  %d = icmp eq i64 %i1, %r\n",
    "  br i1 %d, label %done, label %loop16\n",
    "loop32:\n",
    "  %j = phi i64 [ 0, %pick ], [ %j1, %loop32 ]\n",
    "  %h = getelementptr inbounds i32, ptr %t, i64 %j\n",
    "  store i32 %v32, ptr %h\n",
    "  %j1 = add nuw nsw i64 %j, 1\n",
    "  %c = icmp eq i64 %j1, %r\n",
    "  br i1 %c, label %done, label %loop32\n",
    "done:\n",
    "  ret ptr %q\n",
    "}\n",
);

/// G2 A5: declarations appended after DECLS_U8 only for programs that use `[bool]`.
pub const DECLS_BOOL: &str = "declare void @pruv_check_bool(ptr nocapture readonly, i64) nounwind\n";

/// G2 B2: declarations appended only for programs that use `str` (after DECLS_U8 and after DECLS_BOOL).
pub const DECLS_STR: &str = concat!(
    "declare i32 @pruv_str_boundary(ptr nocapture readonly, i64, i64) nounwind willreturn memory(argmem: read)\n",
    "declare noalias ptr @pruv_substr(ptr nocapture readonly, i64, i64, i64) nounwind\n",
    "declare void @pruv_check_utf8(ptr nocapture readonly, i64) nounwind\n",
);

/// G3 A4: declaration of the representation-check panic, only emitted when some wrapper rejects an invalid enum ordinal.
pub const DECLS_REPR: &str = "declare void @pruv_panic_repr(i64, ptr) noreturn cold nounwind\n";

/// G9 A3: only emitted when the program has a recursive (boxed) edge. The marker global is kept alive through
/// @llvm.used, so linking a runtime without pruv_runtime_nodes_v1 fails even when no node helper survives inlining.
pub const DECLS_NODE: &str = concat!(
    "declare noalias ptr @pruv_node_alloc_v1(i64, i64) nounwind\n",
    "declare noalias ptr @pruv_node_realloc_v1(ptr, i64, i64, i64) nounwind\n",
    "declare void @pruv_node_free_v1(ptr nocapture, i64, i64) nounwind\n",
    "declare void @pruv_runtime_nodes_v1() nounwind\n",
    "@pruv_nodes_marker = internal global ptr @pruv_runtime_nodes_v1\n",
    "@llvm.used = appending global [1 x ptr] [ptr @pruv_nodes_marker], section \"llvm.metadata\"\n",
);

/// Name prefix of a generated generic nominal instance (`pruv_g3_t_...`).
pub const G3_TYPE_PREFIX: &str = "pruv_g3_t_";

/// G3 A4: true for a structured built-in application key `option<X>` / `result<X>` (X != i64); the legacy
/// `option` / `result` constants are not applications.
pub fn is_gopt(t: &str) -> bool {
    t.starts_with("option<") || t.starts_with("result<")
}

/// The argument key X of `option<X>` / `result<X>` (only for a key accepted by `is_gopt`).
pub fn g_arg(t: &str) -> &str {
    g_args(t).0
}

pub fn g_error(t: &str) -> &str {
    g_args(t).1
}

fn g_args(t: &str) -> (&str, &str) {
    let args = &t[7..t.len() - 1];
    if t.starts_with("result<") {
        let mut depth = 0;
        for (i, byte) in args.bytes().enumerate() {
            match byte {
                b'<' => depth += 1,
                b'>' => depth -= 1,
                b',' if depth == 0 => return (&args[..i], &args[i + 1..]),
                _ => {}
            }
        }
    }
    (args, "i64")
}

/// A user struct/enum (including generated generic instances) or a new Option/Result application.
pub fn is_agg(t: &str) -> bool {
    native_vec_elem(t).is_some() || is_gopt(t) || (is_user_type(t) && !crate::gtype::is_g3_key(t))
}

/// Types whose external word form follows the G3 rules (tag normalization, inactive words zero, enum check).
pub fn is_new_app(t: &str) -> bool {
    is_gopt(t) || (is_user_type(t) && t.starts_with(G3_TYPE_PREFIX))
}

const ARRAY_U8: &str = "array_u8";
const ARRAY_BOOL: &str = "array_bool"; // G2 A5
const STR: &str = "str"; // G2 B2

fn is_u8_ty(t: &Option<String>) -> bool {
    t.as_deref() == Some(ARRAY_U8)
}

/// G2 B2: str value (code and contract reads use the same spelling as u8)
fn is_strv(t: &Option<String>) -> bool {
    t.as_deref() == Some(STR)
}

/// G2 B2: whether this is the internal type name of an array or str (`{ ptr, i64 }` representation)
fn is_arr_name(n: &str) -> bool {
    is_array_name(n) || n == STR
}

/// G2 A5: byte arrays (u8, bool, str) — GEP i8, copy/free v2 (1,1)
fn is_bytes_ty(t: &Option<String>) -> bool {
    matches!(t.as_deref(), Some(ARRAY_U8) | Some(ARRAY_BOOL) | Some(STR))
}

/// G2 A5: array type name -> the ty carried in V/Slot (Some only for u8, bool, str)
fn elem_ty(t: &str) -> Option<String> {
    match t {
        ARRAY_U8 => Some(ARRAY_U8.to_string()),
        ARRAY_BOOL => Some(ARRAY_BOOL.to_string()),
        STR => Some(STR.to_string()),
        _ => None,
    }
}

/// Physical element type, independent of the logical array element type.
fn elem_ir(v: &V) -> &'static str {
    if is_bytes_ty(&v.ty) {
        "i8"
    } else {
        match v.layout { "u8" | "i8" => "i8", "u16" | "i16" => "i16", "u32" | "i32" => "i32", _ => "i64" }
    }
}

const NONE_ATTR: &str = "'NoneType' object has no attribute 'name'";
const IDX_ERR: &str = "list index out of range";

// ---------- type helpers ----------

pub fn mangle(n: &str) -> String {
    let mut out = String::new();
    for c in n.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push_str(&format!("${:04x}", c as u32));
        }
    }
    out
}

/// The internal SSA type of a Pruv type
pub fn lty(t: &str) -> String {
    if native_vec_elem(t).is_some() {
        return ARR.to_string();
    }
    if is_collection(t) {
        // G6 A3: opaque handle
        return "ptr".to_string();
    }
    if is_gopt(t) {
        return st(t, false);
    }
    match t {
        "i64" => "i64".to_string(),
        "bool" => "i1".to_string(),
        "unit" => "void".to_string(),
        "array" | "array_u8" | "array_bool" | "str" => ARR.to_string(), // G2 A5, B2
        "option" | "result" => OPT.to_string(),
        _ => format!("%pruv_t_{}", t),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    I64,
    I1,
    I128,
    Arr,
    Opt,
    Unit,
    Agg,
    /// G6 A3: opaque handle of a Vec/HashMap/String box (a = ptr, ty = the collection type)
    Coll,
    /// G9 contract evaluation: a = ptr to the REAL storage of an extended value, ty = its type
    Ref,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Coll => "coll",
            Kind::Ref => "ref",
            Kind::I64 => "i64",
            Kind::I1 => "i1",
            Kind::I128 => "i128",
            Kind::Arr => "arr",
            Kind::Opt => "opt",
            Kind::Unit => "unit",
            Kind::Agg => "agg",
        }
    }
}

pub fn vkind(t: &str) -> Kind {
    if is_collection(t) {
        // G6 A3
        return Kind::Coll;
    }
    if is_gopt(t) {
        return Kind::Agg;
    }
    match t {
        "i64" => Kind::I64,
        "bool" => Kind::I1,
        "unit" => Kind::Unit,
        "array" | "array_u8" | "array_bool" | "str" => Kind::Arr, // G2 A5, B2
        "option" | "result" => Kind::Opt,
        _ => Kind::Agg,
    }
}

/// Storage type of a value inside an aggregate (c: contract/i128 form)
pub fn st(t: &str, c: bool) -> String {
    if native_vec_elem(t).is_some() {
        return ARR.to_string();
    }
    if is_collection(t) {
        return "ptr".to_string();
    }
    if is_gopt(t) {
        let x = st(g_arg(t), c);
        if t.starts_with("option<") {
            return format!("{{ i1, {} }}", x);
        }
        return format!("{{ i1, {}, {} }}", x, st(g_error(t), c));
    }
    match t {
        // LB8 R-6g7-1: the Ok slot of Result<(), E> is empty
        "unit" => "{}".to_string(),
        "i64" => {
            if c {
                "i128".to_string()
            } else {
                "i64".to_string()
            }
        }
        "bool" => "i1".to_string(),
        "option" | "result" => {
            if c {
                COPT.to_string()
            } else {
                OPT.to_string()
            }
        }
        // G9 A3: buffer fields keep the { ptr, i64 } form
        "array" | "array_u8" | "array_bool" | "str" => ARR.to_string(),
        _ => format!("%pruv_{}_{}", if c { "c" } else { "t" }, t),
    }
}

/// G9 field label as in prog.g9.boxed: "Struct.field" or "Enum::Variant.field"
pub fn fld_label(dname: &str, vname: Option<&str>, fname: &str) -> String {
    match vname {
        None => format!("{}.{}", dname, fname),
        Some(v) => format!("{}::{}.{}", dname, v, fname),
    }
}

/// Type table of a program (Python `new_tt`): SCC-condensed declaration order for G9 programs (identical to
/// TypeTable::new otherwise), plus the G9 metadata sets tt.g9_ext (extended declaration names) and tt.g9_boxed.
pub fn new_tt(prog: &Program) -> TypeTable {
    let mut tt = crate::emit_common::make_type_table(prog);
    if let Some(meta) = &prog.g9 {
        tt.g9_ext = meta.extended.iter().cloned().collect();
        tt.g9_boxed = meta.boxed.iter().cloned().collect();
    }
    tt.union_words = enum_union_words(&tt, &abi_layout_types(prog, &tt));
    tt
}

/// Nominal types reachable from a foreign signature or a host ABI entry.
fn abi_layout_types(prog: &Program, tt: &TypeTable) -> HashSet<String> {
    use crate::gtype::{dependencies, GType};
    let explicit = prog.explicit.as_ref();
    let mut work = Vec::new();
    for (i, f) in prog.functions.iter().enumerate() {
        let host = explicit.and_then(|x| x.fns.get(i)).is_some_and(|mk| mk.host_entry);
        if f.foreign.is_some() || is_host_abi_fn(f, tt, explicit.is_some(), host) {
            for ty in f.params.iter().map(|p| &p.ty).chain(std::iter::once(&f.ret)) {
                work.extend(dependencies(&GType::from_key(ty)));
            }
        }
    }
    let mut seen = HashSet::new();
    while let Some(name) = work.pop() {
        if let Some(decl) = tt.decl(&name) {
            if seen.insert(name) {
                work.extend(tt.deps(decl));
            }
        }
    }
    seen
}

const UNION_LIMIT: usize = 1 << 40; // same limit as pruvc/llvmgen.py

fn limited(r: (usize, usize)) -> Option<(usize, usize)> {
    (r.0 <= UNION_LIMIT && r.1 <= UNION_LIMIT).then_some(r)
}

/// Word bounds of code-form enum payloads. Unknown or wider leaves rule out overlap.
/// Each bound is (words, low): words bounds the overlapped size in 8-byte words, low is a lower bound of the
/// flattened size in bytes. An enum overlaps only when 8 * (1 + W) is at most low rounded up to 8, so the union
/// form is never larger than the flattened one on 64-bit targets. Bounds above UNION_LIMIT rule out overlap; sums
/// saturate, so both compilers decide as with exact arithmetic.
fn enum_union_words(tt: &TypeTable, abi_types: &HashSet<String>) -> HashMap<String, usize> {
    fn bound(
        tt: &TypeTable,
        abi_types: &HashSet<String>,
        bounds: &mut HashMap<String, Option<(usize, usize)>>,
        unions: &mut HashMap<String, usize>,
        ty: &str,
    ) -> Option<(usize, usize)> {
        if native_vec_elem(ty).is_some() {
            return Some((3, 24));
        }
        if is_collection(ty) {
            return Some((1, 8));
        }
        if is_gopt(ty) {
            let ok = bound(tt, abi_types, bounds, unions, g_arg(ty))?;
            let err = if ty.starts_with("result<") {
                bound(tt, abi_types, bounds, unions, g_error(ty))?
            } else {
                (0, 0)
            };
            return limited((1 + ok.0 + err.0, 1 + ok.1 + err.1));
        }
        match ty {
            "i64" => return Some((1, 8)),
            "bool" => return Some((1, 1)),
            "option" | "result" => return Some((2, 2)),
            "array" | "array_u8" | "array_bool" | "str" => return Some((3, 24)),
            _ => {}
        }
        if let Some(&words) = bounds.get(ty) {
            return words;
        }
        let d = tt.decl(ty)?;
        // Recursive references in the emitted types go through boxes or vector descriptors.
        bounds.insert(ty.to_string(), None);
        let mut fields_bound = |fields: &[FieldDecl], variant| {
            let mut sum: (usize, usize) = (0, 0);
            for f in fields {
                let b = if tt.g9_boxed.contains(&fld_label(d.name(), variant, &f.name)) {
                    (1, 8)
                } else {
                    bound(tt, abi_types, bounds, unions, &f.ty)?
                };
                sum = (sum.0.saturating_add(b.0), sum.1.saturating_add(b.1));
            }
            limited(sum)
        };
        let total = match d {
            TypeDecl::Struct(s) => fields_bound(&s.fields, None),
            TypeDecl::Enum(e) => {
                let mut count = 0;
                let mut sum: (usize, usize) = (0, 0);
                let mut max = 1;
                for v in e.variants.iter().filter(|v| !v.fields.is_empty()) {
                    let b = fields_bound(&v.fields, Some(&v.name))?;
                    count += 1;
                    sum = (sum.0.saturating_add(b.0), sum.1.saturating_add(b.1));
                    max = max.max(b.0);
                }
                let flat_low = sum.1.saturating_add(15) / 8 * 8;
                if count >= 2 && !abi_types.contains(ty) && 8 * (1 + max) <= flat_low {
                    unions.insert(ty.to_string(), max);
                    limited((1 + max, 8 * (1 + max)))
                } else {
                    limited((sum.0.saturating_add(1), sum.1.saturating_add(8)))
                }
            }
        };
        bounds.insert(ty.to_string(), total);
        total
    }

    let mut bounds = HashMap::new();
    let mut unions = HashMap::new();
    for d in tt.ordered() {
        bound(tt, abi_types, &mut bounds, &mut unions, d.name());
    }
    unions
}

/// t is an extended user type (G9): recursive, or holding a buffer or an extended type
pub fn is_ext_t(tt: &TypeTable, t: &str) -> bool {
    native_vec_elem(t).is_some() || is_owned_result(t) || tt.g9_ext.contains(t)
}

/// G5 A3: some parameter has the inout mode (a mutable borrow of caller storage)
pub fn has_inout(f: &Function) -> bool {
    f.params.iter().any(|p| p.mode_pos.is_some())
}

pub fn has_take_array(f: &Function) -> bool {
    f.params.iter().any(|p| p.take && is_array_name(&p.ty))
}

/// Legacy functions with extended, inout, or take-array parameters use only internal entry points.
/// R-6g9-1: a borrowed result (a { ptr, i64 } view the host would read as an owned array) is internal too
pub fn is_internal_fn(f: &Function, tt: &TypeTable) -> bool {
    has_ext_sig(f, tt) || has_inout(f) || has_take_array(f) || view_ret(f)
}

/// An extended (G9) type in the signature (separate from the inout rule: an explicit host entry may have inout params)
pub fn has_ext_sig(f: &Function, tt: &TypeTable) -> bool {
    f.params.iter().any(|p| is_ext_t(tt, &p.ty)) || is_ext_t(tt, &f.ret)
}

/// internal LLVM result type: an extended (G9) aggregate result goes through explicit output storage
pub fn ir_ret(tt: &TypeTable, f: &Function) -> String {
    if is_ext_t(tt, &f.ret) {
        "void".to_string()
    } else if view_ret(f) {
        VIEW.to_string()
    } else {
        lty(&f.ret)
    }
}

/// G9: some struct field / enum payload has the internal type `name` (array_u8, array_bool, str)
pub fn type_has_field(prog: &Program, name: &str) -> bool {
    prog.types.iter().any(|d| d.all_fields().iter().any(|f| f.ty == name))
}

fn t_decl<'t>(tt: &'t TypeTable, ty: &str) -> R<&'t TypeDecl> {
    tt.decl(ty).ok_or_else(|| format!("'{}'", ty))
}

fn t_fields<'t>(tt: &'t TypeTable, ty: &str, variant: Option<&str>) -> R<&'t [FieldDecl]> {
    tt.fields(ty, variant).ok_or_else(|| format!("'{}'", variant.unwrap_or(ty)))
}

fn t_field_index(tt: &TypeTable, ty: &str, name: &str, variant: Option<&str>) -> R<usize> {
    tt.field_index(ty, name, variant).ok_or_else(|| format!("'{}' is not in list", name))
}

fn t_variant_index(tt: &TypeTable, ty: &str, variant: &str) -> R<usize> {
    tt.variant_index(ty, variant).ok_or_else(|| format!("'{}' is not in list", variant))
}

/// Storage index of the i-th field of `variant` in a flattened code or contract enum.
fn flattened_payload_index(tt: &TypeTable, ty: &str, variant: &str, i: usize) -> R<usize> {
    // The variant -> base offset map is computed once per (type table, type) and memoized: rescanning the variants on
    // every call made helper generation quadratic in the number of variants. The cache lives in `words_memo` (the
    // type table's existing per-type memo) under keys that cannot collide with a type key (they start with NUL).
    let key = |v: &str| format!("\u{0}payload\u{0}{}\u{0}{}", ty, v);
    if let Some(&b) = tt.words_memo.borrow().get(&key(variant)) {
        return Ok(b + i);
    }
    if let TypeDecl::Enum(e) = t_decl(tt, ty)? {
        let mut memo = tt.words_memo.borrow_mut();
        let mut base = 1usize;
        let mut found: Option<usize> = None;
        for v in &e.variants {
            memo.insert(key(&v.name), base);
            if v.name == variant && found.is_none() {
                found = Some(base);
            }
            base += v.fields.len();
        }
        if let Some(b) = found {
            return Ok(b + i);
        }
    } else {
        return Err("'StructDecl' object has no attribute 'variants'".to_string());
    }
    Err(format!("'{}'", variant))
}

fn variant_type(tt: &TypeTable, ty: &str, variant: Option<&str>, c: bool) -> String {
    if let Some(v) = variant.filter(|_| !c && tt.union_words.contains_key(ty)) {
        return format!("%pruv_v_{}__{}", ty, v);
    }
    st(ty, c)
}

fn field_index(tt: &TypeTable, ty: &str, variant: Option<&str>, i: usize, c: bool) -> R<usize> {
    match variant {
        None => Ok(i),
        Some(_) if !c && tt.union_words.contains_key(ty) => Ok(1 + i),
        Some(v) => flattened_payload_index(tt, ty, v, i),
    }
}

/// W(T): number of i64 words in the external (pruv_abi_*) form.  Memoized per type table (a nested generic value
/// would otherwise recompute every subtree: quadratic).
pub fn words(tt: &TypeTable, ty: &str) -> R<usize> {
    if let Some(&n) = tt.words_memo.borrow().get(ty) {
        return Ok(n);
    }
    let r = if is_gopt(ty) {
        // G3: 1 + W(X) (+ W(E) for Result)
        let x = words(tt, g_arg(ty))?;
        1 + x + if ty.starts_with("result<") { words(tt, g_error(ty))? } else { 0 }
    } else {
        match ty {
            "i64" | "bool" => 1,
            // LB8 R-6g7-1: the Ok payload of Result<(), E> has no words
            "unit" => 0,
            "option" | "result" => 2,
            _ => match t_decl(tt, ty)? {
                TypeDecl::Struct(s) => {
                    let mut sum = 0usize;
                    for f in &s.fields {
                        sum += words(tt, &f.ty)?;
                    }
                    sum.max(1)
                }
                TypeDecl::Enum(e) => {
                    let mut sum = 1usize;
                    for v in &e.variants {
                        for f in &v.fields {
                            sum += words(tt, &f.ty)?;
                        }
                    }
                    sum
                }
            },
        }
    };
    tt.words_memo.borrow_mut().insert(ty.to_string(), r);
    Ok(r)
}

pub fn type_defs(tt: &TypeTable) -> String {
    let mut out: Vec<String> = Vec::new();
    for c in [false, true] {
        for d in tt.ordered() {
            if c && tt.g9_ext.contains(d.name()) {
                continue; // G9: an extended value has no i128 contract mirror
            }
            // a boxed (recursive SCC) field is an opaque ptr to an independently allocated node
            let fs_of = |fields: &[FieldDecl], vname: Option<&str>| -> Vec<String> {
                fields
                    .iter()
                    .map(|f| {
                        if !tt.g9_boxed.is_empty() && tt.g9_boxed.contains(&fld_label(d.name(), vname, &f.name)) {
                            "ptr".to_string()
                        } else {
                            st(&f.ty, c)
                        }
                    })
                    .collect()
            };
            let fs: Vec<String> = match d {
                TypeDecl::Struct(s) => fs_of(&s.fields, None),
                TypeDecl::Enum(e) => {
                    let mut v = vec!["i64".to_string()];
                    if let Some(w) = tt.union_words.get(d.name()).filter(|_| !c) {
                        v.push(format!("[{} x i64]", w));
                        for var in e.variants.iter().filter(|var| !var.fields.is_empty()) {
                            let variant = Some(var.name.as_str());
                            let vt = variant_type(tt, d.name(), variant, c);
                            let fields = fs_of(&var.fields, variant);
                            out.push(format!("{} = type {{ i64, {} }}", vt, fields.join(", ")));
                        }
                    } else {
                        for var in &e.variants {
                            v.extend(fs_of(&var.fields, Some(var.name.as_str())));
                        }
                    }
                    v
                }
            };
            let body = if fs.is_empty() { "{}".to_string() } else { format!("{{ {} }}", fs.join(", ")) };
            out.push(format!("%pruv_{}_{} = type {}", if c { "c" } else { "t" }, d.name(), body));
        }
    }
    let mut s = out.join("\n");
    if !out.is_empty() {
        s.push('\n');
    }
    s
}

// ---------- values and slots ----------

/// SSA value. Meaning by k: i64|i1|i128, arr(a=ptr,b=len), opt(a=tag,b=payload), unit, agg(a=aggregate SSA, ty=type).
/// A Slot has the same shape (arr: a=ptr slot, b=len slot).
#[derive(Clone, Debug)]
pub struct V {
    pub layout: &'static str,
    /// P3 batch 2: declared value range of the array elements (None = unknown)
    pub rng: Option<(i128, i128)>,
    pub k: Kind,
    pub a: String,
    pub b: String,
    pub ty: Option<String>,
    /// G9 (agg only): address of storage that holds this value when known (a parameter, a local slot, a boxed node,
    /// a call-result slot); lets a read argument pass the pointer instead of spilling a copy.
    pub p: Option<String>,
    /// P3: (owner slot, variant, field index, owner dead) for a match payload binder
    pub mv: Option<Box<(Slot, String, usize, bool)>>,
    /// P3 (arr only): allocated element count (cap) SSA for a value, cap storage for a Slot; None means cap == len (b)
    pub c: Option<String>,
}

pub type Slot = V;

/// P3: the allocated element count of an owned array value (the size argument of every free)
pub fn capof(v: &V) -> String {
    v.c.clone().unwrap_or_else(|| v.b.clone())
}

impl V {
    pub fn new(k: Kind, a: impl Into<String>) -> V {
        V { layout: "i64", rng: None, k, a: a.into(), b: "None".to_string(), ty: None, p: None, mv: None, c: None }
    }
    pub fn with_b(k: Kind, a: impl Into<String>, b: impl Into<String>) -> V {
        V { layout: "i64", rng: None, k, a: a.into(), b: b.into(), ty: None, p: None, mv: None, c: None }
    }
    pub fn agg(a: impl Into<String>, ty: &str) -> V {
        V { layout: "i64", rng: None, k: Kind::Agg, a: a.into(), b: "None".to_string(), ty: Some(ty.to_string()), p: None, mv: None, c: None }
    }
    /// an aggregate value with a known storage address
    pub fn agg_p(a: impl Into<String>, ty: &str, p: impl Into<String>) -> V {
        V { layout: "i64", rng: None, k: Kind::Agg, a: a.into(), b: "None".to_string(), ty: Some(ty.to_string()), p: Some(p.into()), mv: None, c: None }
    }
    pub fn unit() -> V {
        V::new(Kind::Unit, "None")
    }
}

/// Name binding: ("ssa", V) or ("slot", Slot)
#[derive(Clone, Debug)]
pub enum Bind {
    Ssa(V),
    Slot(Slot),
}

impl Bind {
    fn val(&self) -> &V {
        match self {
            Bind::Ssa(v) | Bind::Slot(v) => v,
        }
    }
}

#[derive(Default)]
struct Scope {
    names: HashMap<String, Bind>,
    arrays: Vec<Slot>,
}

pub struct Mod {
    owned_collection_fields: bool,
    vec_elems: BTreeSet<String>,
    foreign_requires: bool,
    vec_get_checked: bool,
    vec_get_used: bool,
    pub(crate) storage: StoragePlan, // pub(crate): abi.rs reads the return layout for the shrink size
    strs: HashMap<String, String>,
    order: Vec<String>,
    /// G3: a wrapper rejects an invalid enum ordinal through pruv_panic_repr
    pub uses_repr: bool,
    par_types: Vec<String>,
    par_trampolines: Vec<String>,
    /// P10H: `<pkg>__` (or "") in front of every program-specific global symbol (`sym_prefix`)
    pub sym_pre: String,
    /// G6 A3: shim symbol -> declare line (iterated sorted by symbol)
    pub g6_decls: BTreeMap<String, String>,
    /// G6 A3: string_to_str needs the v2 copy/free declarations
    pub g6_v2: bool,
    /// P3 M3: some string_to_str takes the String buffer (pruv_g6_v1_string_take)
    pub g6_take: bool,
    /// P3 M5: trusted String functions lowered through the unboxed adapter <f>__str (sorted by name)
    pub g6_unbox: BTreeSet<String>,
    /// G6 A3: some function references @pruv_g6_bridge_v1
    pub g6_bridge: bool,
    /// G9 A3: types whose runtime-contract size / equality helpers are needed
    pub need_size: BTreeSet<String>,
    pub need_eq: BTreeSet<String>,
    /// P2 batch 4: widths of the llvm.bswap declarations the byte-word accesses used
    pub bswap: BTreeSet<u32>,
    pub uses_utf8_valid: bool,
    pub uses_utf8_panic: bool,
    /// P3: some external array-return wrapper normalizes cap to len with @pruv_shrink_v2
    pub shrink: bool,
    /// some host wrapper of an inout entry calls @pruv_overlap (module gets DECLS_OVERLAP)
    pub overlap: bool,
    /// P3-S1: the stack plan (set by gen_llvm; empty = every array on the heap)
    pub stack: crate::stack::StackPlan,
    /// P3-S1: some stack array calls @pruv_fill_v2
    pub fill: bool,
    /// P3: tail-filled alloc helpers used (pruv_alloc_tail, pruv_alloc_tail_u8)
    pub tail_i64: bool,
    pub tail_u8: bool,
    /// P05G: resize helpers used (pruv_resize, pruv_resize_u8, pruv_resize_narrow)
    pub resize_i64: bool,
    pub resize_u8: bool,
    pub resize_narrow: bool,
    /// LG32 (G-DB52-4): in-place resize helpers used (pruv_regrow, pruv_regrow_u8)
    pub raw_regrow: bool,
    pub regrow_i64: bool,
    pub regrow_u8: bool,
    /// LG34 (G-LG32-3): the narrow-layout regrow helper (pruv_regrow_narrow)
    pub regrow_narrow: bool,
    /// P17a: some eq_range lowers to a memcmp call (declare i32 @memcmp)
    pub memcmp: bool,
    /// LB6 MC: some find_byte lowers to a memchr call (declare ptr @memchr)
    pub memchr: bool,
    /// P3 batch 2: `!range` node bodies in first-use order (index = metadata number)
    pub range_nodes: Vec<String>,
    /// G4 A3: explicit visibility mode (Pruv calls use the Rust-backend routing)
    pub explicit: bool,
    /// G4 A3: function name (stem) -> source-qualified identity (Python `f.disp`), explicit mode only
    pub fn_disp: HashMap<String, String>,
    resize_params: HashSet<(String, usize)>,
}

impl Mod {
    pub fn new() -> Mod {
        Mod {
            owned_collection_fields: false,
            vec_elems: BTreeSet::new(),
            foreign_requires: false,
            vec_get_checked: false,
            vec_get_used: false,
            storage: StoragePlan::default(),
            strs: HashMap::new(),
            order: Vec::new(),
            uses_repr: false,
            par_types: Vec::new(),
            par_trampolines: Vec::new(),
            sym_pre: String::new(),
            g6_decls: BTreeMap::new(),
            g6_v2: false,
            g6_take: false,
            g6_unbox: BTreeSet::new(),
            g6_bridge: false,
            need_size: BTreeSet::new(),
            need_eq: BTreeSet::new(),
            bswap: BTreeSet::new(),
            uses_utf8_valid: false,
            uses_utf8_panic: false,
            shrink: false,
            overlap: false,
            stack: crate::stack::StackPlan::default(),
            fill: false,
            tail_i64: false,
            tail_u8: false,
            resize_i64: false,
            resize_u8: false,
            resize_narrow: false,
            raw_regrow: false,
            regrow_i64: false,
            regrow_u8: false,
            regrow_narrow: false,
            memcmp: false,
            memchr: false,
            range_nodes: Vec::new(),
            explicit: false,
            fn_disp: HashMap::new(),
            resize_params: HashSet::new(),
        }
    }

    pub fn string(&mut self, text: &str) -> String {
        if let Some(n) = self.strs.get(text) {
            return n.clone();
        }
        let n = format!("@.str.{}", self.order.len());
        self.strs.insert(text.to_string(), n.clone());
        self.order.push(text.to_string());
        n
    }

    pub fn render_strings(&self) -> String {
        let mut out: Vec<String> = Vec::new();
        for text in &self.order {
            let b = text.as_bytes();
            let mut body = String::new();
            for &c in b {
                if (32..127).contains(&c) && c != 34 && c != 92 {
                    body.push(c as char);
                } else {
                    body.push_str(&format!("\\{:02X}", c));
                }
            }
            out.push(format!("{} = private unnamed_addr constant [{} x i8] c\"{}\\00\"", self.strs[text], b.len() + 1, body));
        }
        let mut s = out.join("\n");
        if !out.is_empty() {
            s.push('\n');
        }
        s
    }
}

impl Default for Mod {
    fn default() -> Self {
        Mod::new()
    }
}

/// These inout parameters borrow the mutable slot itself.
fn inout_slot_param(param: &Param, resize: bool) -> bool {
    param.mode_pos.is_some() && (matches!(param.ty.as_str(), "i64" | "bool")
        || is_agg(&param.ty) || (is_array_name(&param.ty) && resize))
}

/// Internal parameter list [(type, name)]
/// G9: an extended result is the leading `ptr %out` (output storage); an extended parameter is `ptr %p.NAME` to valid
/// caller-owned storage (see sig_text for its attributes).
pub fn ir_params(f: &Function, tt: &TypeTable, resizes: &HashSet<(String, usize)>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if is_ext_t(tt, &f.ret) {
        out.push(("ptr".to_string(), "%out".to_string()));
    }
    for (i, p) in f.params.iter().enumerate() {
        if is_ext_t(tt, &p.ty) || inout_slot_param(p, resizes.contains(&(f.name.clone(), i))) {
            out.push(("ptr".to_string(), format!("%p.{}", mangle(&p.name))));
            continue;
        }
        let n = mangle(&p.name);
        if is_arr_name(&p.ty) {
            out.push(("ptr".to_string(), format!("%p.{}.a", n)));
            out.push(("i64".to_string(), format!("%p.{}.n", n)));
            if p.take && is_array_name(&p.ty) {
                out.push(("i64".to_string(), format!("%p.{}.c", n)));
            }
        } else if p.ty == "bool" {
            out.push(("i1".to_string(), format!("%p.{}", n)));
        } else if p.ty == "i64" {
            out.push(("i64".to_string(), format!("%p.{}", n)));
        } else if is_collection(&p.ty) {
            // G6 A3: plain opaque handle %p.NAME.h (no readonly/noalias attributes)
            out.push(("ptr".to_string(), format!("%p.{}.h", n)));
        } else if is_agg(&p.ty) {
            out.push((lty(&p.ty), format!("%p.{}", n)));
        } else {
            out.push((OPT.to_string(), format!("%p.{}", n)));
        }
    }
    out
}

/// Array and extended aggregate borrows are readonly unless marked inout.
/// Owned array pointers and collection handles can escape through the function result.
pub fn sig_text(params: &[(String, String)], f: Option<&Function>) -> String {
    let mut_names: Vec<_> = f.into_iter().flat_map(|f| &f.params)
        .filter(|p| p.mode_pos.is_some())
        .flat_map(|p| {
            let base = format!("%p.{}", mangle(&p.name));
            [format!("{base}.a"), base]
        })
        .collect();
    let take_names: Vec<_> = f.into_iter().flat_map(|f| &f.params)
        .filter(|p| p.take && (is_array_name(&p.ty) || is_agg(&p.ty) || native_vec_elem(&p.ty).is_some()))
        // LG9: a take record parameter is owned record storage
        .map(|p| if is_array_name(&p.ty) { format!("%p.{}.a", mangle(&p.name)) } else { format!("%p.{}", mangle(&p.name)) })
        .collect();
    let read_vec_names: Vec<_> = f.into_iter().flat_map(|f| &f.params)
        .filter(|p| native_vec_elem(&p.ty).is_some())
        .map(|p| format!("%p.{}", mangle(&p.name)))
        .collect();
    // R-6g9-1: the data pointer of a `from` source escapes through a view result
    let escapes: Vec<_> = f.into_iter().filter(|f| view_ret(f))
        .flat_map(|f| f.borrow_from.iter().flatten())
        .map(|n| format!("%p.{}.a", mangle(n)))
        .collect();
    params
        .iter()
        .map(|(t, n)| {
            format!(
                "{} {}",
                if t == "ptr" && n != "%out" && !n.ends_with(".h") && !take_names.contains(n) {
                    let esc = escapes.contains(n);
                    if mut_names.contains(n) {
                        if esc { "ptr noalias" } else { "ptr noalias nocapture" }
                    } else if read_vec_names.contains(n) {
                        "ptr nocapture readonly"
                    } else if esc {
                        "ptr noalias readonly"
                    } else {
                        "ptr noalias nocapture readonly"
                    }
                } else {
                    t.as_str()
                },
                n
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The noreturn call made from a guard's failure block (counterpart of a Python lambda). String constants are registered at call time.
pub enum Fail {
    Cost,
    Panic1(&'static str, String),
    Oob(String, String),
    Utf8(String),
    // LG25: re-enter pruv_substr (ptr, len, lo, hi), which panics with the matching message
    Substr(String, String, String, String),
    Requires(String, String),
    /// u8 element range violation: (value SSA, range text). The string is registered when the fail block is emitted.
    Elem(String, &'static str),
    VecGet,
    VecPush { elem: String, drop_element: bool, panic: &'static str },
}

pub(crate) fn expr_ty(e: &Expr) -> &Option<String> {
    match e {
        Expr::Spent(x) => &x.ty,
        Expr::IntLit(x) => &x.ty,
        Expr::BoolLit(x) => &x.ty,
        Expr::StrLit(x) => &x.ty,
        Expr::Var(x) => &x.ty,
        Expr::Unary(x) => &x.ty,
        Expr::Binary(x) => &x.ty,
        Expr::Call(x) => &x.ty,
        Expr::Index(x) => &x.ty,
        Expr::Ctor(x) => &x.ty,
        Expr::Quant(x) => &x.ty,
        Expr::StructLit(x) => &x.ty,
        Expr::EnumCtor(x) => &x.ty,
        Expr::FieldAccess(x) => &x.ty,
        Expr::IsVariant(x) => &x.ty,
        Expr::VariantField(x) => &x.ty,
        Expr::Move(x) => &x.ty,
        Expr::InoutArg(x) => &x.ty,
    }
}

fn arg(a: &[Expr], i: usize) -> R<&Expr> {
    a.get(i).ok_or_else(|| IDX_ERR.to_string())
}

fn need_ty(t: &Option<String>) -> R<&str> {
    t.as_deref().ok_or_else(|| NONE_ATTR.to_string())
}

/// P3: the cap storage of an array slot
fn need_c(c: &Option<String>) -> R<&str> {
    c.as_deref().ok_or_else(|| NONE_ATTR.to_string())
}

pub struct FnGen<'a> {
    m: &'a mut Mod,
    tt: &'a TypeTable,
    funcs: &'a HashMap<String, &'a Function>,
    f: &'a Function,
    verified: bool,
    /// LG35: the str_from_utf8 call whose bytes the enclosing `if` checked (codegen::utf8_checked)
    utf8_ok: usize,
    /// LG35: locals borrowed as the scrutinee of an enclosing match arm (codegen's pinned)
    pinned: HashSet<String>,
    cost: bool,
    cost_bound: String,
    lines: Vec<String>,
    allocas: Vec<String>,
    nt: usize,
    nl: usize,
    nslot: usize,
    /// R-6g9-1: a `ref` local exists, so no last-use copy or resize takes its source
    views: bool,
    nq: usize,
    cur: String,
    dead: bool,
    scopes: Vec<Scope>,
    // Bindings and cleanup lists clone slots; retain layout by unique slot address.
    resize_slots: HashSet<String>,
    slot_layouts: HashMap<String, &'static str>,
    slot_rngs: HashMap<String, Option<(i128, i128)>>,
    temps: Vec<Vec<V>>,
    cenv: HashMap<String, V>,
    budget: Option<String>,
    /// G6 A3: this function references @pruv_g6_bridge_v1
    bridge: bool,
    /// G9 A3: the parameter names of the function (Python `s.cparams`, set when the contract parameters are bound)
    cparams: HashSet<String>,
    /// G9: constructor temporaries (type, storage) of the contract expression being evaluated, per region
    cregions: Vec<Vec<(String, String)>>,
    /// P3: return-movable names (see ret_movable)
    ret_cand: HashSet<String>,
    /// P3: owner slot address -> None (whole value moved) or (variant, {indices of moved fields})
    ret_moved: HashMap<String, Option<(String, HashSet<usize>)>>,
    /// P3: enclosing blocks being lowered: (names declared so far, names named later, loop body)
    frames: Vec<(HashSet<String>, HashSet<String>, bool, bool)>,
    /// P3: names the copy of the let / assignment being lowered moves (move_src)
    moving: HashSet<String>,
    /// P3 M3: String locals whose string_to_str in the return result position takes the buffer (ret_takable)
    ret_take: HashSet<String>,
    /// P3 M3: the String local the let / assignment being lowered takes (take_src)
    take_let: HashSet<String>,
    /// P3 M5: the let being lowered binds an unboxed String local (unbox_let)
    unbox_next: bool,
    /// P3: the let being lowered is a tail-filled alloc (tail_fill): the address of its alloc call and the bound k
    tail: Option<(usize, Expr)>,
}

impl<'a> FnGen<'a> {
    pub fn new(
        m: &'a mut Mod,
        tt: &'a TypeTable,
        funcs: &'a HashMap<String, &'a Function>,
        f: &'a Function,
        verified: bool,
    ) -> FnGen<'a> {
        let resize_slots = crate::check::resize_slot_names(f, &m.resize_params);
        FnGen {
            m,
            tt,
            funcs,
            f,
            verified,
            utf8_ok: 0,
            pinned: HashSet::new(),
            cost: !verified && f.cost.is_some(),
            cost_bound: String::new(),
            lines: Vec::new(),
            allocas: Vec::new(),
            nt: 0,
            nl: 0,
            nslot: 0,
            views: false,
            nq: 0,
            cur: "None".to_string(),
            dead: false,
            scopes: Vec::new(),
            resize_slots,
            slot_layouts: HashMap::new(),
            slot_rngs: HashMap::new(),
            temps: Vec::new(),
            cenv: HashMap::new(),
            budget: None,
            bridge: false,
            cparams: HashSet::new(),
            cregions: Vec::new(),
            ret_cand: HashSet::new(),
            ret_moved: HashMap::new(),
            frames: Vec::new(),
            moving: HashSet::new(),
            ret_take: HashSet::new(),
            take_let: HashSet::new(),
            unbox_next: false,
            tail: None,
        }
    }

    // ---------- emission ----------
    pub fn ins(&mut self, text: String) -> R<()> {
        if self.dead {
            return Err(format!("llvmgen: internal error: instruction after terminator: {}", text));
        }
        self.lines.push(format!("  {}", text));
        Ok(())
    }

    pub fn term(&mut self, text: String) -> R<()> {
        self.ins(text)?;
        self.dead = true;
        Ok(())
    }

    fn cost_fail(&mut self) -> R<()> {
        self.ins("call void @pruv_panic_cost()".into())
    }

    fn cost_add(&mut self, units: &str) -> R<()> {
        let old = self.op("load i128, ptr %cost.spent".into())?;
        let pair = self.op(format!("call {{ i128, i1 }} @llvm.uadd.with.overflow.i128(i128 {}, i128 {})", old, units))?;
        let value = self.op(format!("extractvalue {{ i128, i1 }} {}, 0", pair))?;
        let overflow = self.op(format!("extractvalue {{ i128, i1 }} {}, 1", pair))?;
        self.guard(&overflow, Fail::Cost, true)?;
        self.ins(format!("store i128 {}, ptr %cost.spent", value))
    }

    fn cost_call(&mut self, name: &str, args: &[String]) -> R<()> {
        let bound = self.op(format!("call i128 @{}__pruv_cost({})", mangle(name), args.join(", ")))?;
        let negative = self.op(format!("icmp slt i128 {}, 0", bound))?;
        self.guard(&negative, Fail::Cost, true)?;
        // Nonnegative i128 bound plus one always fits u128.
        let units = self.op(format!("add i128 {}, 1", bound))?;
        self.cost_add(&units)
    }

    fn cost_check(&mut self) -> R<()> {
        let negative = self.op(format!("icmp slt i128 {}, 0", self.cost_bound))?;
        let spent = self.op("load i128, ptr %cost.spent".into())?;
        let exceeded = self.op(format!("icmp ugt i128 {}, {}", spent, self.cost_bound))?;
        let bad = self.op(format!("or i1 {}, {}", negative, exceeded))?;
        self.guard(&bad, Fail::Cost, true)
    }

    fn parallel(&mut self, st: &Par) -> R<()> {
        // Entry storage can be reused after the synchronous bridge joins.
        let jobs = self.tmp();
        let jt = format!("[{} x %pruv_g7_job]", st.calls.len());
        self.allocas.push(format!("{} = alloca {}", jobs, jt));
        for (i, call) in st.calls.iter().enumerate() {
            let Expr::Call(call) = call else { return Err("invalid par call".into()); };
            let f = self.funcs[&call.name];
            let mut fields = Vec::new();
            let mut cost_args = Vec::new();
            for (j, x) in call.args.iter().enumerate() {
                let x = if let Expr::InoutArg(x) = x {
                    if inout_slot_param(&f.params[j], self.m.resize_params.contains(&(f.name.clone(), j))) {
                        let ptr = self.inout_place_ptr(&x.expr)?;
                        if self.cost { cost_args.push(format!("ptr {ptr}")); }
                        fields.push(("ptr", ptr));
                        continue;
                    }
                    &*x.expr
                } else { x };
                let v = self.ev(x, false)?;
                if self.cost {
                    if v.k == Kind::Arr {
                        cost_args.extend([format!("ptr {}", v.a), format!("i64 {}", v.b)]);
                    } else {
                        cost_args.push(format!("{} {}", v.k.name(), v.a));
                    }
                }
                if v.k == Kind::Arr {
                    fields.push(("ptr", v.a));
                    fields.push(("i64", v.b));
                } else if v.k == Kind::I1 {
                    fields.push(("i8", self.op(format!("zext i1 {} to i8", v.a))?));
                } else {
                    fields.push((v.k.name(), v.a));
                }
            }
            if self.cost { self.cost_call(&call.name, &cost_args)?; }
            let n = self.m.par_types.len();
            let ct = format!("%pruv_g7_ctx_{}", n);
            let cb = format!("@pruv_g7_cb_{}", n);
            self.m.par_types.push(format!("{} = type {{ {} }}", ct, fields.iter().map(|(t, _)| *t).collect::<Vec<_>>().join(", ")));
            let ctx = self.tmp();
            self.allocas.push(format!("{} = alloca {}", ctx, ct));
            let mut tr = vec![format!("define private void {}(ptr %ctx) {{", cb), "entry:".into()];
            let mut args = Vec::new();
            for (j, (ty, value)) in fields.iter().enumerate() {
                let ptr = self.op(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 {}", ct, ctx, j))?;
                self.ins(format!("store {} {}, ptr {}", ty, value, ptr))?;
                tr.push(format!("  %p{} = getelementptr inbounds {}, ptr %ctx, i32 0, i32 {}", j, ct, j));
                tr.push(format!("  %v{} = load {}, ptr %p{}", j, ty, j));
                if *ty == "i8" {
                    tr.push(format!("  %b{} = icmp ne i8 %v{}, 0", j, j));
                    args.push(format!("i1 %b{}", j));
                } else {
                    args.push(format!("{} %v{}", ty, j));
                }
            }
            let target = if !self.verified && !f.requires.is_empty() { "par_chk" } else { "body" };
            tr.push(format!("  call void @{}__pruv_{}({})", mangle(&f.name), target, args.join(", ")));
            tr.push("  ret void".into());
            tr.push("}".into());
            self.m.par_trampolines.push(tr.join("\n") + "\n");
            for (j, value) in [&cb, &ctx].iter().enumerate() {
                let ptr = self.op(format!("getelementptr inbounds {}, ptr {}, i32 0, i64 {}, i32 {}", jt, jobs, i, j))?;
                self.ins(format!("store ptr {}, ptr {}", value, ptr))?;
            }
        }
        self.ins(format!("call void @pruv_g7_v1_run(ptr {}, i64 {})", jobs, st.calls.len()))
    }

    pub fn tmp(&mut self) -> String {
        self.nt += 1;
        format!("%t{}", self.nt)
    }

    pub fn op(&mut self, text: String) -> R<String> {
        let r = self.tmp();
        self.ins(format!("{} = {}", r, text))?;
        Ok(r)
    }

    pub fn label(&mut self, hint: &str) -> String {
        self.nl += 1;
        format!("{}.{}", hint, self.nl)
    }

    pub fn place(&mut self, lbl: &str) {
        self.lines.push(format!("{}:", lbl));
        self.cur = lbl.to_string();
        self.dead = false;
    }

    pub fn br(&mut self, l: &str) -> R<()> {
        self.term(format!("br label %{}", l))
    }

    pub fn condbr(&mut self, c: &str, a: &str, b: &str) -> R<()> {
        self.term(format!("br i1 {}, label %{}, label %{}", c, a, b))
    }

    fn run_fail(&mut self, f: &Fail) -> R<()> {
        match f {
            Fail::Cost => self.cost_fail(),
            Fail::VecPush { elem, drop_element, panic } => {
                self.ins(format!("call void @pruv_vdrop__{elem}(ptr %v)"))?;
                if *drop_element {
                    self.ins(format!("call void @pruv_drop__{elem}(ptr %x)"))?;
                }
                self.ins((*panic).to_string())
            }
            Fail::VecGet => {
                for region in self.temps.clone().iter().rev() {
                    for value in region.iter().rev() {
                        self.free_val(value)?;
                    }
                }
                for i in (0..self.scopes.len()).rev() {
                    let owners = self.scopes[i].arrays.clone();
                    self.free_scope(&owners)?;
                }
                let requires = if self.m.foreign_requires { "1" } else { "0" };
                self.m.vec_get_used = true;
                self.ins(format!("call void @pruv_panic_vec_get_v1(i64 {requires})"))
            }
            Fail::Panic1(func, msg) => {
                let s = self.m.string(msg);
                self.ins(format!("call void @{}(ptr {})", func, s))
            }
            Fail::Utf8(n) => self.ins(format!("call void @pruv_panic_utf8(i64 {})", n)),
            Fail::Substr(p, n, lo, hi) => self.ins(format!("call ptr @pruv_substr(ptr {}, i64 {}, i64 {}, i64 {})", p, n, lo, hi)),
            Fail::Oob(i, n) => self.ins(format!("call void @pruv_panic_oob(i64 {}, i64 {})", i, n)),
            Fail::Requires(a, b) => self.ins(format!("call void @pruv_panic_requires(ptr {}, ptr {})", a, b)),
            Fail::Elem(v, range) => {
                let s = self.m.string(range);
                self.ins(format!("call void @pruv_panic_elem(i64 {}, ptr {})", v, s))
            }
        }
    }

    /// Continue only when cond != fail_when; otherwise call fail in the failure block
    pub fn guard(&mut self, cond: &str, fail: Fail, fail_when: bool) -> R<()> {
        let ok = self.label("ok");
        let bad = self.label("fail");
        if fail_when {
            self.condbr(cond, &bad, &ok)?;
        } else {
            self.condbr(cond, &ok, &bad)?;
        }
        self.place(&bad);
        self.run_fail(&fail)?;
        self.term("unreachable".to_string())?;
        self.place(&ok);
        Ok(())
    }

    pub fn render(&mut self, header: &str) -> String {
        let mut out: Vec<String> = vec![format!("{} {{", header), "entry:".to_string()];
        for a in &self.allocas {
            out.push(format!("  {}", a));
        }
        out.push("  br label %start".to_string());
        if self.bridge {
            // G6 A3: a (volatile) read of the shim's bridge marker keeps the reference alive: linking without the shim
            // fails. lines[0] is the `start:` label.
            if let Some(first) = self.lines.first() {
                out.push(first.clone());
            }
            out.push(format!("  %g6.bridge = load volatile i8, ptr @{}pruv_g6_bridge_v1", self.m.sym_pre));
            out.extend(self.lines.iter().skip(1).cloned());
            self.m.g6_bridge = true;
        } else {
            out.extend(self.lines.iter().cloned());
        }
        out.push("}".to_string());
        let mut s = out.join("\n");
        s.push('\n');
        s
    }

    // ---------- slots / scopes ----------
    fn array_slot(&mut self, a: String, ty: &str) -> R<Slot> {
        let b = g9_gep(self, ARR, &a, 1)?;
        let c = g9_gep(self, ARR, &a, 2)?;
        let mut slot = V::with_b(Kind::Arr, a, b);
        slot.c = Some(c);
        slot.ty = elem_ty(ty);
        Ok(slot)
    }

    /// R-6g9-1: a `ref [u8]` / `ref str` local: data and length only (no cap), never in a scope's owner list
    fn view_slot(&mut self, name: &str, ty: &str) -> Slot {
        let base = format!("%l{}.{}", self.nslot, mangle(name));
        self.nslot += 1;
        self.views = true;
        let a = format!("{}.a", base);
        let b = format!("{}.b", base);
        self.allocas.push(format!("{} = alloca ptr", a));
        self.allocas.push(format!("{} = alloca i64", b));
        let mut sl = V::with_b(Kind::Arr, a, b);
        sl.ty = elem_ty(ty);
        sl
    }

    pub fn new_slot(&mut self, name: &str, ty: &str) -> R<Slot> {
        let kind = vkind(ty);
        let base = format!("%l{}.{}", self.nslot, mangle(name));
        self.nslot += 1;
        let a = format!("{}.a", base);
        let b = format!("{}.b", base);
        Ok(match kind {
            Kind::I64 => {
                self.allocas.push(format!("{} = alloca i64", a));
                V::new(Kind::I64, a)
            }
            Kind::I1 => {
                self.allocas.push(format!("{} = alloca i1", a));
                V::new(Kind::I1, a)
            }
            Kind::Coll => {
                // G6 A3: owner slot, null-initialized
                self.allocas.push(format!("{} = alloca ptr", a));
                self.allocas.push(format!("store ptr null, ptr {}", a));
                V { layout: "i64", rng: None, k: Kind::Coll, a, b: "None".to_string(), ty: Some(ty.to_string()), p: None, mv: None, c: None }
            }
            Kind::Arr if self.resize_slots.contains(name) => {
                self.allocas.push(format!("{a} = alloca {ARR}"));
                // The packed slot's len / cap addresses are taken where the slot is made (as the Python backend does).
                return self.array_slot(a, ty);
            }
            Kind::Arr => {
                self.allocas.push(format!("{} = alloca ptr", a));
                self.allocas.push(format!("{} = alloca i64", b));
                let mut sl = V::with_b(Kind::Arr, a, b);
                sl.ty = elem_ty(ty); // G2 A5: Some only for u8 and bool, None otherwise (default unchanged)
                let c = format!("{}.c", base); // P3: the cap lives next to the len
                self.allocas.push(format!("{} = alloca i64", c));
                sl.c = Some(c);
                sl
            }
            Kind::Agg => {
                self.allocas.push(format!("{} = alloca {}", a, lty(ty)));
                V::agg(a, ty)
            }
            _ => {
                self.allocas.push(format!("{} = alloca i1", a));
                self.allocas.push(format!("{} = alloca i64", b));
                V::with_b(Kind::Opt, a, b)
            }
        })
    }

    pub fn load(&mut self, sl: &Slot) -> R<V> {
        match sl.k {
            Kind::I64 | Kind::I1 => {
                let r = self.op(format!("load {}, ptr {}", sl.k.name(), sl.a))?;
                Ok(V::new(sl.k, r))
            }
            Kind::Agg => {
                let ty = need_ty(&sl.ty)?.to_string();
                let r = self.op(format!("load {}, ptr {}", lty(&ty), sl.a))?;
                Ok(V::agg_p(r, &ty, sl.a.clone()))
            }
            Kind::Coll => {
                let r = self.op(format!("load ptr, ptr {}", sl.a))?;
                Ok(V { layout: "i64", rng: None, k: Kind::Coll, a: r, b: "None".to_string(), ty: sl.ty.clone(), p: None, mv: None, c: None })
            }
            Kind::Arr => {
                let p = self.op(format!("load ptr, ptr {}", sl.a))?;
                let n = self.op(format!("load i64, ptr {}", sl.b))?;
                let mut v = V::with_b(Kind::Arr, p, n);
                if let Some(cp) = &sl.c {
                    // a view slot (R-6g9-1) has no cap
                    v.c = Some(self.op(format!("load i64, ptr {}", cp))?);
                }
                v.ty = sl.ty.clone();
                v.layout = self.slot_layouts.get(&sl.a).copied().unwrap_or(sl.layout);
                v.rng = self.slot_rngs.get(&sl.a).copied().unwrap_or(sl.rng);
                Ok(v)
            }
            _ => {
                let t = self.op(format!("load i1, ptr {}", sl.a))?;
                let v = self.op(format!("load i64, ptr {}", sl.b))?;
                Ok(V::with_b(Kind::Opt, t, v))
            }
        }
    }

    pub fn store(&mut self, sl: &Slot, v: &V) -> R<()> {
        match sl.k {
            Kind::I64 | Kind::I1 => self.ins(format!("store {} {}, ptr {}", sl.k.name(), v.a, sl.a)),
            Kind::Agg => {
                let ty = need_ty(&sl.ty)?.to_string();
                self.ins(format!("store {} {}, ptr {}", lty(&ty), v.a, sl.a))
            }
            Kind::Coll => self.ins(format!("store ptr {}, ptr {}", v.a, sl.a)),
            Kind::Arr => {
                self.slot_layouts.insert(sl.a.clone(), v.layout);
                self.slot_rngs.insert(sl.a.clone(), v.rng);
                self.ins(format!("store ptr {}, ptr {}", v.a, sl.a))?;
                self.ins(format!("store i64 {}, ptr {}", v.b, sl.b))?;
                match &sl.c {
                    Some(cp) => self.ins(format!("store i64 {}, ptr {}", capof(v), cp)),
                    None => Ok(()), // R-6g9-1: a view slot has no cap
                }
            }
            _ => {
                self.ins(format!("store i1 {}, ptr {}", v.a, sl.a))?;
                self.ins(format!("store i64 {}, ptr {}", v.b, sl.b))
            }
        }
    }

    pub fn lookup(&self, name: &str) -> R<Bind> {
        for sc in self.scopes.iter().rev() {
            if let Some(b) = sc.names.get(name) {
                return Ok(b.clone());
            }
        }
        Err(format!("llvmgen: unknown variable {}", name))
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(Scope::default());
    }

    pub fn free_val(&mut self, v: &V) -> R<()> {
        // The *_nn wrappers let the optimizer drop the call when the moved-out slot is null.
        if v.k == Kind::Coll {
            // G6 A3: a fresh (non-null) collection temporary
            return self.g6_drop(v);
        }
        if v.k == Kind::Agg {
            // G9 A3: an extended temporary
            return self.drop_agg(v);
        }
        if v.layout != "i64" {
            return self.ins(format!("call void @pruv_free_v2_nn(ptr {}, i64 {}, i64 {}, i64 {})", v.a, capof(v), elem_size(v.layout), alignment(v.layout)));
        }
        if is_bytes_ty(&v.ty) {
            return self.ins(format!("call void @pruv_free_v2_nn(ptr {}, i64 {}, i64 1, i64 1)", v.a, capof(v)));
        }
        self.ins(format!("call void @pruv_free_nn(ptr {}, i64 {})", v.a, capof(v))) // P3: the size is the cap, not the len
    }

    fn free_scope(&mut self, arrays: &[Slot]) -> R<()> {
        for sl in arrays.iter().rev() {
            if let Some(mv) = self.ret_moved.get(&sl.a).cloned() {
                self.drop_rest(sl, &mv)?;
            } else if sl.k == Kind::Coll {
                self.drop_slot(sl)?;
            } else if sl.k == Kind::Agg {
                // G9 A3: an owned extended local
                self.drop_agg_slot(need_ty(&sl.ty)?, &sl.a)?;
            } else {
                let v = self.load(sl)?;
                self.free_val(&v)?;
            }
        }
        Ok(())
    }

    /// P3: release an owned extended slot whose value (moved is None) or some unboxed payload fields of its
    /// current variant (moved = {field index}) a return moved out: only the parts that were not moved
    fn drop_rest(&mut self, sl: &Slot, moved: &Option<(String, HashSet<usize>)>) -> R<()> {
        let (vn, idxs) = match moved {
            None => return Ok(()),
            Some(m) => m,
        };
        let tt = self.tt;
        let ty = need_ty(&sl.ty)?.to_string();
        let fds = t_fields(tt, &ty, Some(vn))?;
        for (i, fd) in fds.iter().enumerate() {
            let ft = fd.ty.as_str();
            if idxs.contains(&i) || !(is_arr_name(ft) || is_collection(ft) || self.is_ext(ft)) {
                continue;
            }
            let gp = g9_field_gep(self, &ty, &sl.a, Some(vn), i)?;
            if self.fb(&ty, Some(vn), &fd.name) {
                let p = self.op(format!("load ptr, ptr {}", gp))?;
                self.ins(format!("call void @pruv_dropnode__{}(ptr {})", ft, p))?;
            } else if is_arr_name(ft) {
                let x = self.op(format!("load {}, ptr {}", ARR, gp))?;
                let v = self.unpack(&x, ft, false)?;
                self.free_val(&v)?;
            } else if is_collection(ft) {
                self.drop_slot(&V { ty: Some(ft.to_string()), ..V::new(Kind::Coll, gp) })?;
            } else {
                self.drop_agg_slot(ft, &gp)?;
            }
        }
        Ok(())
    }

    fn owned_slot(&self, sl: &Slot) -> bool {
        self.scopes.iter().any(|sc| sc.arrays.iter().any(|x| x.a == sl.a))
    }

    /// P3 M3: `name` is an owned local String (not a parameter, not borrowed): string_to_str of it may take the buffer
    fn takeable(&self, name: &str) -> bool {
        let Ok(Bind::Slot(x)) = self.lookup(name) else { return false };
        if !self.owned_slot(&x) {
            return false;
        }
        let is_string = x.k == Kind::Coll && matches!(&x.ty, Some(t) if crate::emit_common::key_name(t) == "string");
        // P3 M5: an unboxed String local is an owned str array slot
        let is_str = x.k == Kind::Arr && x.ty.as_deref() == Some(STR);
        if !(is_string || is_str) {
            return false;
        }
        // scope 0 holds the owned (take) parameters
        !self.scopes.first().is_some_and(|sc| sc.arrays.iter().any(|y| y.a == x.a))
    }

    /// P3 M3: the String locals whose `string_to_str(x)` is in a result position of the returned expression (the
    /// expression itself, or a field of a constructor / struct literal / enum constructor in it, the same place walk
    /// as ret_movable) and that the expression names exactly once; the return is their last use, so the call takes the
    /// buffer and the return's cleanup skips the slot
    fn ret_takable(&self, e: &Expr) -> HashSet<String> {
        let mut out = HashSet::new();
        if !self.may_take() {
            return out;
        }
        fn walk(x: &Expr, counts: &mut Vec<(String, usize)>) {
            let n = match x {
                Expr::Var(v) => Some(&v.name),
                Expr::Move(v) => Some(&v.name),
                _ => None,
            };
            if let Some(n) = n {
                match counts.iter_mut().find(|(m, _)| m == n) {
                    Some(c) => c.1 += 1,
                    None => counts.push((n.clone(), 1)),
                }
            }
            for k in crate::refine::kids(x) {
                walk(k, counts);
            }
        }
        let mut counts = Vec::new();
        walk(e, &mut counts);
        fn place(x: &Expr, res: &mut Vec<String>) {
            match x {
                Expr::Ctor(_) | Expr::StructLit(_) | Expr::EnumCtor(_) => {
                    for k in crate::refine::kids(x) {
                        place(k, res);
                    }
                }
                Expr::Call(c) if c.name == "string_to_str" && c.args.len() == 1 => {
                    if let Expr::Var(v) = &c.args[0] {
                        res.push(v.name.clone());
                    }
                }
                _ => {}
            }
        }
        let mut res = Vec::new();
        place(e, &mut res);
        for n in res {
            if counts.iter().any(|(m, k)| *m == n && *k == 1) && self.takeable(&n) {
                out.insert(n);
            }
        }
        out
    }

    /// P3 M3: a let / assignment `y = string_to_str(x)` whose whole right side is that call, at the last use of the
    /// owned local String x (dead_after), takes x's buffer; returns {x}, else an empty set
    fn take_src(&self, name: &str, e: &Expr) -> HashSet<String> {
        let mut out = HashSet::new();
        if !self.may_take() {
            return out;
        }
        let Expr::Call(c) = e else { return out };
        if c.name != "string_to_str" || c.args.len() != 1 {
            return out;
        }
        let Expr::Var(x) = &c.args[0] else { return out };
        if x.name == name || !self.dead_after(&x.name) || !self.takeable(&x.name) {
            return out;
        }
        out.insert(x.name.clone());
        out
    }

    /// P3 M5: every occurrence of `name` in the expression e is len(name) or string_to_str(name) (code only)
    fn unbox_uses(name: &str, e: &Expr, contract: bool) -> bool {
        match e {
            Expr::Var(v) => return v.name != name,
            Expr::Move(v) => return v.name != name,
            Expr::Quant(q) if q.var == name => return false,
            Expr::Call(c) => {
                if c.name == name {
                    return false;
                }
                if (c.name == "len" || c.name == "string_to_str") && c.args.len() == 1 {
                    if let Expr::Var(v) = &c.args[0] {
                        if v.name == name {
                            return !contract;
                        }
                    }
                }
            }
            _ => {}
        }
        crate::refine::kids(e).into_iter().all(|k| Self::unbox_uses(name, k, contract))
    }

    /// P3 M5: no statement of the block (nested blocks included) binds, assigns or uses `name` other than through
    /// len(name) / string_to_str(name); a contract expression (assert, loop invariant / decreases) never names it
    fn unbox_block(name: &str, stmts: &[Stmt]) -> bool {
        for st in stmts {
            let ok = match st {
                Stmt::Let(x) => x.name != name && Self::unbox_uses(name, &x.value, false),
                Stmt::Assign(x) => x.name != name && Self::unbox_uses(name, &x.value, false),
                Stmt::FieldAssign(x) => x.name != name && Self::unbox_uses(name, &x.value, false),
                Stmt::IndexAssign(x) => {
                    x.name != name && Self::unbox_uses(name, &x.index, false) && Self::unbox_uses(name, &x.value, false)
                }
                Stmt::If(x) => {
                    Self::unbox_uses(name, &x.cond, false) && Self::unbox_block(name, &x.then) && Self::unbox_block(name, &x.els)
                }
                Stmt::While(x) => {
                    Self::unbox_uses(name, &x.cond, false)
                        && x.invariants.iter().all(|i| Self::unbox_uses(name, i, true))
                        && Self::unbox_uses(name, &x.decreases, true)
                        && Self::unbox_block(name, &x.body)
                }
                Stmt::ForMap(_) => false,
                Stmt::Return(x) => x.value.as_ref().map_or(true, |v| Self::unbox_uses(name, v, false)),
                Stmt::Assert(x) => Self::unbox_uses(name, &x.cond, true),
                Stmt::ExprStmt(x) => Self::unbox_uses(name, &x.expr, false),
                Stmt::Par(x) => x.calls.iter().all(|c| Self::unbox_uses(name, c, false)),
                Stmt::Match(x) => {
                    Self::unbox_uses(name, &x.scrutinee, false)
                        && x.arms.iter().all(|a| !a.binders.iter().any(|b| b == name) && Self::unbox_block(name, &a.body))
                }
            };
            if !ok {
                return false;
            }
        }
        true
    }

    /// P3: `let mut x: [i64 | u8] = alloc(n, 0); let mut i: i64 = 0; while i < k { x[i] = e; i = i + 1; }` in
    /// verified mode (not cost mode), with k a name other than x and i and e a call-free expression not naming x:
    /// the loop overwrites x[0..min(k, n)) before any read, so the alloc zeroes only the rest. Returns the alloc
    /// call's address and the k node
    fn tail_fill(&self, x: &Let, nxt: &[Stmt]) -> Option<(usize, Expr)> {
        let is0 = |e: &Expr, v: i128| matches!(e, Expr::IntLit(l) if l.value.parse::<i128>().ok() == Some(v));
        let is_var = |e: &Expr, n: &str| matches!(e, Expr::Var(v) if v.name == n);
        if !self.verified || self.cost || !x.mutable {
            return None;
        }
        let tn = crate::emit_common::key_name(&x.ty);
        if tn != "array" && tn != "array_u8" {
            return None;
        }
        let Expr::Call(c) = &x.value else { return None };
        if c.name != "alloc" || self.funcs.contains_key(&c.name) || c.args.len() != 2 || !is0(&c.args[1], 0) {
            return None;
        }
        // P3: statements x[0] = e0; x[1] = e1; ... (constant indices in order, each e call-free and not naming x)
        // overwrite x[0..j) before any read; the run's length is the bound
        let mut j = 0usize;
        while j < nxt.len() {
            match &nxt[j] {
                Stmt::IndexAssign(sa) if sa.name == x.name && is0(&sa.index, j as i128) && tail_pure(&sa.value, &x.name) => j += 1,
                _ => break,
            }
        }
        if j > 0 {
            let Expr::IntLit(z) = &c.args[1] else { return None };
            let mut kl = z.clone();
            kl.value = j.to_string();
            return Some((c as *const Call as usize, Expr::IntLit(kl)));
        }
        if nxt.len() < 2 {
            return None;
        }
        let Stmt::Let(li) = &nxt[0] else { return None };
        if crate::emit_common::key_name(&li.ty) != "i64" || !is0(&li.value, 0) || li.name == x.name {
            return None;
        }
        let (xn, i) = (x.name.as_str(), li.name.as_str());
        let Stmt::While(w) = &nxt[1] else { return None };
        let Expr::Binary(cb) = &w.cond else { return None };
        let k = match &*cb.right {
            Expr::Var(v) if v.name != xn && v.name != i && v.name != "result" => &*cb.right,
            _ => return None,
        };
        if cb.op != "<" || !is_var(&cb.left, i) || w.body.len() != 2 {
            return None;
        }
        let Stmt::IndexAssign(sa) = &w.body[0] else { return None };
        if sa.name != xn || !is_var(&sa.index, i) || !tail_pure(&sa.value, xn) {
            return None;
        }
        let Stmt::Assign(inc) = &w.body[1] else { return None };
        let Expr::Binary(ib) = &inc.value else { return None };
        if inc.name != i || ib.op != "+" || !is_var(&ib.left, i) || !is0(&ib.right, 1) {
            return None;
        }
        Some((c as *const Call as usize, k.clone()))
    }

    /// P3 M5: `let x: String = f(args)` with a trusted foreign f returning String (not a catalog op, not cost mode)
    /// whose name is, in the rest of the block, only measured (len) or converted (string_to_str): x is kept as an owned
    /// str array filled by the adapter pruv_g6_v1_<f>__str, with no String box
    fn unbox_let(&self, x: &Let, rest: &[Stmt]) -> bool {
        if self.cost || crate::emit_common::key_name(&x.ty) != "string" {
            return false;
        }
        let Expr::Call(c) = &x.value else { return false };
        let Some(f) = self.funcs.get(&c.name) else { return false };
        if f.foreign.is_none()
            || crate::foreign::is_catalog(&c.name)
            || !is_collection(&f.ret)
            || crate::emit_common::key_name(&f.ret) != "string"
        {
            return false;
        }
        Self::unbox_block(&x.name, rest)
    }

    /// P3 (the LLVM form of the Rust backend's return_movable / consuming match): a return is the last use of every
    /// local, so an owned extended local, or an unboxed extended payload binder of a match on one, that the returned
    /// expression names exactly once (counting every name rooted at the same owner) moves out in an owning position
    /// instead of being cloned; the cleanup of that return then releases only what was not moved (drop_rest).
    fn ret_movable(&self, e: &Expr) -> HashSet<String> {
        let mut out = HashSet::new();
        if self.cost {
            return out;
        }
        fn walk(x: &Expr, counts: &mut Vec<(String, usize)>) {
            let n = match x {
                Expr::Var(v) => Some(&v.name),
                Expr::Move(v) => Some(&v.name),
                _ => None,
            };
            if let Some(n) = n {
                match counts.iter_mut().find(|(m, _)| m == n) {
                    Some(c) => c.1 += 1,
                    None => counts.push((n.clone(), 1)),
                }
            }
            for k in crate::refine::kids(x) {
                walk(k, counts);
            }
        }
        let mut counts = Vec::new();
        walk(e, &mut counts);
        // only a name in a result position (the returned expression itself, or a constructor / struct literal field
        // of one) moves: its value then lands in the returned value on every path, never in a temporary that a
        // nested short-circuit or call releases before the return ends
        fn place(x: &Expr, res: &mut HashSet<String>) {
            match x {
                Expr::Var(v) => {
                    res.insert(v.name.clone());
                }
                Expr::Move(v) => {
                    res.insert(v.name.clone());
                }
                Expr::Ctor(_) | Expr::StructLit(_) | Expr::EnumCtor(_) => {
                    for k in crate::refine::kids(x) {
                        place(k, res);
                    }
                }
                Expr::Call(c) if c.name == "copy" && c.args.len() == 1 => {
                    // copy(x) of an owned local array / array binder moves x instead
                    if let Expr::Var(v) = &c.args[0] {
                        res.insert(v.name.clone());
                    }
                }
                Expr::Call(c) if c.name == "truncate" && c.args.len() == 2 => {
                    // truncate(x, m) of an owned local array shrinks x in place instead
                    if let Expr::Var(v) = &c.args[0] {
                        res.insert(v.name.clone());
                    }
                }
                _ => {}
            }
        }
        let mut res = HashSet::new();
        place(e, &mut res);
        // root owner address -> [(name, candidate, the owner itself)]
        let mut roots: Vec<(String, Vec<(String, bool, bool)>)> = Vec::new();
        let mut add = |a: &str, n: &str, c: bool, whole: bool| match roots.iter_mut().find(|(r, _)| r == a) {
            Some(r) => r.1.push((n.to_string(), c, whole)),
            None => roots.push((a.to_string(), vec![(n.to_string(), c, whole)])),
        };
        for (n, _) in &counts {
            match self.lookup(n) {
                Ok(Bind::Slot(x)) => {
                    if (x.k == Kind::Agg && self.is_ext_opt(&x.ty) || x.k == Kind::Arr) && self.owned_slot(&x) {
                        add(&x.a, n, true, true);
                    } else if matches!(x.k, Kind::Agg | Kind::Coll | Kind::Arr) {
                        add(&x.a, n, false, true);
                    }
                }
                Ok(Bind::Ssa(x)) => {
                    if let Some(mv) = &x.mv {
                        add(&mv.0.a, n, true, false);
                    }
                }
                Err(_) => {}
            }
        }
        for (_a, ns) in &roots {
            // the owner named itself moves only alone; distinct payload binders of one owner are distinct fields
            if ns.len() > 1 && ns.iter().any(|x| x.2) {
                continue;
            }
            for (n, c, _) in ns {
                if *c && res.contains(n) && counts.iter().any(|(m, k)| m == n && *k == 1) {
                    out.insert(n.clone());
                }
            }
        }
        out
    }

    // ---------- G9 A3: extended values (ownership helpers) ----------
    fn is_ext(&self, t: &str) -> bool {
        is_ext_t(self.tt, t)
    }

    fn is_ext_opt(&self, t: &Option<String>) -> bool {
        match t {
            Some(t) => is_ext_t(self.tt, t),
            None => false,
        }
    }

    /// The field `fname` (of `variant` for an enum) of the user type ty is a boxed recursive edge.
    fn fb(&self, ty: &str, variant: Option<&str>, fname: &str) -> bool {
        if self.tt.g9_boxed.is_empty() || !crate::emit_common::is_user_key(ty) {
            return false;
        }
        match self.tt.decl(ty) {
            Some(d) => self.tt.g9_boxed.contains(&fld_label(d.name(), variant, fname)),
            None => false,
        }
    }

    /// Address of the storage that holds the aggregate v: its known address, else a fresh entry-block copy.
    fn spill(&mut self, v: &V) -> R<String> {
        if let Some(p) = &v.p {
            return Ok(p.clone());
        }
        let ty = need_ty(&v.ty)?.to_string();
        let base = format!("%sp{}", self.nslot);
        self.nslot += 1;
        self.allocas.push(format!("{} = alloca {}", base, lty(&ty)));
        self.ins(format!("store {} {}, ptr {}", lty(&ty), v.a, base))?;
        Ok(base)
    }

    /// Fresh entry-block output storage for an aggregate result.
    fn new_out(&mut self, ty: &str) -> String {
        let base = format!("%o{}", self.nslot);
        self.nslot += 1;
        self.allocas.push(format!("{} = alloca {}", base, lty(ty)));
        base
    }

    /// A fresh extended value: owned by the destination, or a temporary dropped at the end of the statement.
    fn fresh(&mut self, v: V, owned: bool) -> R<V> {
        if !owned {
            self.temps.last_mut().ok_or_else(|| IDX_ERR.to_string())?.push(v.clone());
        }
        Ok(v)
    }

    /// Deep copy of the extended value v (a view) into new owned storage.
    fn clone_val(&mut self, v: &V) -> R<V> {
        let src = self.spill(v)?;
        let ty = need_ty(&v.ty)?.to_string();
        let dst = self.new_out(&ty);
        let sym = if let Some(elem) = native_vec_elem(&ty) {
            self.m.vec_elems.insert(elem.to_string());
            format!("pruv_vclone__{elem}")
        } else {
            format!("pruv_clone__{ty}")
        };
        self.ins(format!("call void @{sym}(ptr {src}, ptr {dst})"))?;
        let r = self.op(format!("load {}, ptr {}", lty(&ty), dst))?;
        Ok(V::agg_p(r, &ty, dst))
    }

    fn drop_agg(&mut self, v: &V) -> R<()> {
        let ty = need_ty(&v.ty)?.to_string();
        let p = self.spill(v)?;
        self.drop_agg_slot(&ty, &p)
    }

    fn drop_agg_slot(&mut self, ty: &str, p: &str) -> R<()> {
        if is_owned_result(ty) {
            let slot = g9_gep(self, &lty(ty), p, 2)?;
            let handle = self.op(format!("load ptr, ptr {slot}"))?;
            return self.drop_handle(&handle);
        }
        if let Some(elem) = native_vec_elem(ty) {
            self.m.vec_elems.insert(elem.to_string());
            return self.ins(format!("call void @pruv_vdrop__{elem}(ptr {p})"));
        }
        self.ins(format!("call void @pruv_drop__{ty}(ptr {p})"))
    }

    fn drop_handle(&mut self, handle: &str) -> R<()> {
        let nonnull = self.op(format!("icmp ne ptr {handle}, null"))?;
        let drop = self.label("gdrop");
        let done = self.label("gdone");
        self.condbr(&nonnull, &drop, &done)?;
        self.place(&drop);
        let mut value = V::new(Kind::Coll, handle);
        value.ty = Some("string".to_string());
        self.g6_drop(&value)?;
        self.br(&done)?;
        self.place(&done);
        Ok(())
    }

    fn mk_octor(&mut self, e: &Ctor) -> R<V> {
        let ty = need_ty(&e.ty)?;
        let layout = st(ty, false);
        if e.name == "Ok" && e.arg.is_none() {
            // LB8 R-6g7-1: Ok(()) of Result<(), String> sets only the tag
            return Ok(V::agg(self.op(format!("insertvalue {layout} zeroinitializer, i1 true, 0"))?, ty));
        }
        let arg = e.arg.as_deref().ok_or_else(|| IDX_ERR.to_string())?;
        if e.name == "Ok" {
            let value = self.ev(arg, false)?;
            let tag = self.op(format!("insertvalue {layout} zeroinitializer, i1 true, 0"))?;
            return Ok(V::agg(self.put(&tag, ty, 1, &value, g_arg(ty), false)?, ty));
        }
        let slot = if let Expr::Var(var) = arg {
            match self.lookup(&var.name)? {
                Bind::Slot(slot) => Some(slot),
                Bind::Ssa(_) => return Err(e2_diag(e.line, e.col, &format!(
                    "Err of the borrowed String {} needs the Rust backend", var.name))),
            }
        } else {
            None
        };
        let value = self.ev(arg, true)?;
        if let Some(slot) = slot {
            self.ins(format!("store ptr null, ptr {}", slot.a))?;
        }
        Ok(V::agg(self.op(format!("insertvalue {layout} zeroinitializer, ptr {}, 2", value.a))?, ty))
    }

    fn owned_match(&mut self, m: &Match) -> R<()> {
        let scrutinee = &m.scrutinee;
        let ty = need_ty(expr_ty(scrutinee))?;
        if let Expr::Var(var) = scrutinee {
            if !matches!(self.lookup(&var.name)?, Bind::Slot(_)) {
                return Err(e2_diag(m.line, m.col, &format!(
                    "a match on the borrowed Result<_, String> {} needs the Rust backend", var.name)));
            }
        }
        self.begin_temps();
        let value = self.ev(scrutinee, false)?;
        for temp in self.temps.pop().ok_or_else(|| IDX_ERR.to_string())?.iter().rev() {
            if temp.k != value.k || temp.a != value.a {
                self.free_val(temp)?;
            }
        }
        let layout = lty(ty);
        let tag = self.op(format!("extractvalue {layout} {}, 0", value.a))?;
        // LB8 R-6g7-1: Result<(), String> has no Ok payload to read
        let ok = if g_arg(ty) == "unit" { None } else { Some(self.get(&value, ty, 1, g_arg(ty), false)?) };
        let handle = self.op(format!("extractvalue {layout} {}, 2", value.a))?;
        if matches!(scrutinee, Expr::Var(_)) {
            let slot = g9_gep(self, &layout, value.p.as_deref().ok_or_else(|| IDX_ERR.to_string())?, 2)?;
            self.ins(format!("store ptr null, ptr {slot}"))?;
        }
        let yes = self.label("armT");
        let no = self.label("armF");
        let end = self.label("endmatch");
        self.condbr(&tag, &yes, &no)?;
        let mut reach = false;
        for (ctor, label) in [("Ok", yes), ("Err", no)] {
            let arm = m.arms.iter().find(|arm| arm.ctor == ctor).ok_or_else(|| IDX_ERR.to_string())?;
            self.place(&label);
            let mut bind = Vec::new();
            let mut own = None;
            if let Some(name) = arm.binders.first() {
                let binding = if ctor == "Ok" {
                    Bind::Ssa(ok.clone().ok_or_else(|| IDX_ERR.to_string())?)
                } else {
                    let slot = self.new_slot(name, "string")?;
                    self.ins(format!("store ptr {handle}, ptr {}", slot.a))?;
                    own = Some(slot.clone());
                    Bind::Slot(slot)
                };
                bind.push((name.clone(), binding));
            } else if ctor == "Err" {
                self.drop_handle(&handle)?;
            }
            if self.run_block_with_owner(&arm.body, bind, false, own)? {
                self.br(&end)?;
                reach = true;
            }
        }
        if reach {
            self.place(&end);
        } else {
            self.dead = true;
        }
        Ok(())
    }

    /// Address of the storage of an extended expression in a READ position (argument): a local slot or a parameter is
    /// passed as is, a fresh temporary lives (registered in self.temps) until the end of the statement.
    fn view_ptr(&mut self, e: &Expr) -> R<String> {
        if let Expr::Var(x) = e {
            return match self.lookup(&x.name)? {
                Bind::Slot(s) => Ok(s.a),
                Bind::Ssa(v) => self.spill(&v),
            };
        }
        let v = self.ev(e, false)?;
        self.spill(&v)
    }

    /// Address of a mutable place in caller storage.
    fn inout_place_ptr(&mut self, e: &Expr) -> R<String> {
        match e {
            Expr::Var(x) => match self.lookup(&x.name)? {
                Bind::Slot(s) => Ok(s.a),
                Bind::Ssa(_) => Err(format!("llvmgen: no mutable storage for {}", x.name)),
            },
            Expr::FieldAccess(x) => {
                let ty = need_ty(expr_ty(&x.base))?;
                let p = self.inout_place_ptr(&x.base)?;
                let i = t_field_index(self.tt, ty, &x.field, None)?;
                let g = g9_gep(self, &lty(ty), &p, i)?;
                if self.fb(ty, None, &x.field) {
                    return self.op(format!("load ptr, ptr {}", g));
                }
                Ok(g)
            }
            _ => Err("llvmgen: expected a mutable place".into()),
        }
    }

    /// The storage a `move d.a` empties: the slot of a record local, or (LB9 R-6g8-4) the payload storage of a struct
    /// binder of a consuming match, which the scrutinee's drop then finds emptied.
    fn field_move_root(&mut self, x: &Move) -> R<Slot> {
        let bound = self.lookup(&x.name)?;
        let Bind::Ssa(v) = &bound else {
            return Ok(bound.val().clone());
        };
        let v = v.clone();
        let base = if let Some(p) = &v.p {
            p.clone() // a boxed payload: the binder views the child node
        } else if let Some(mv) = &v.mv {
            // an inline payload of the owned scrutinee slot
            let (sl, vn, k, _dead) = (**mv).clone();
            g9_field_gep(self, need_ty(&sl.ty)?, &sl.a, Some(&vn), k)?
        } else {
            return Err(e2_diag(x.line, x.col, &format!(
                "moving a field of the match binder {} needs the Rust backend", x.name)));
        };
        Ok(V { k: Kind::Agg, a: base, p: None, mv: None, ..v })
    }

    /// LG4: (address of the field reached by `path` from the extended root slot `sl`, its declared type); a boxed
    /// intermediate edge is loaded (as `inout_place_ptr` does).
    fn field_ptr_ty(&mut self, sl: &Slot, path: &[String]) -> R<(String, String)> {
        let tt = self.tt;
        let mut cur = sl.a.clone();
        let mut cty = need_ty(&sl.ty)?.to_string();
        for fname in path {
            let i = t_field_index(tt, &cty, fname, None)?;
            let g = g9_gep(self, &lty(&cty), &cur, i)?;
            let boxed = self.fb(&cty, None, fname);
            let fds = t_fields(tt, &cty, None)?;
            let next = fds.get(i).ok_or_else(|| IDX_ERR.to_string())?.ty.clone();
            cur = if boxed { self.op(format!("load ptr, ptr {}", g))? } else { g };
            cty = next;
        }
        Ok((cur, cty))
    }

    /// Field value of the aggregate `base` (type ty); a boxed field is a VIEW of the child node
    /// (a shallow load, never dropped through this value).
    fn getf(&mut self, base: &V, ty: &str, i: usize, fd: &FieldDecl, variant: Option<&str>, c: bool) -> R<V> {
        let boxed = !c && self.fb(ty, variant, &fd.name);
        if !c && variant.is_some() && self.tt.union_words.contains_key(ty) {
            let p = self.spill(base)?;
            let field = g9_field_gep(self, ty, &p, variant, i)?;
            if native_vec_elem(&fd.ty).is_some() {
                return self.load(&V::agg(field, &fd.ty));
            }
            let ft = if boxed { "ptr".to_string() } else { st(&fd.ty, false) };
            let x = self.op(format!("load {}, ptr {}", ft, field))?;
            if boxed {
                let r = self.op(format!("load {}, ptr {}", lty(&fd.ty), x))?;
                return Ok(V::agg_p(r, &fd.ty, x));
            }
            return self.unpack(&x, &fd.ty, false);
        }
        let idx = field_index(self.tt, ty, variant, i, c)?;
        if boxed {
            let x = self.op(format!("extractvalue {} {}, {}", st(ty, c), base.a, idx))?;
            let r = self.op(format!("load {}, ptr {}", lty(&fd.ty), x))?;
            return Ok(V::agg_p(r, &fd.ty, x));
        }
        if !c && native_vec_elem(&fd.ty).is_some() {
            if let Some(p) = &base.p {
                let field = g9_gep(self, &lty(ty), p, idx)?;
                return self.load(&V::agg(field, &fd.ty));
            }
        }
        self.get(base, ty, idx, &fd.ty, c)
    }

    /// insertvalue of the owned field value v; a boxed field first moves into a new node.
    fn putf(&mut self, acc: &str, ty: &str, i: usize, v: &V, fd: &FieldDecl, variant: Option<&str>, c: bool) -> R<String> {
        let t = variant_type(self.tt, ty, variant, c);
        let idx = field_index(self.tt, ty, variant, i, c)?;
        if !c && self.fb(ty, variant, &fd.name) {
            let nd = self.op(format!("call ptr @pruv_box__{}({} {})", fd.ty, lty(&fd.ty), v.a))?;
            return self.op(format!("insertvalue {} {}, ptr {}, {}", t, acc, nd, idx));
        }
        let x = self.pack(v, &fd.ty, c)?;
        self.op(format!("insertvalue {} {}, {} {}, {}", t, acc, st(&fd.ty, c), x, idx))
    }

    fn finish_enum(&mut self, mut acc: String, ty: &str, variant: Option<&str>, c: bool) -> R<V> {
        let t = st(ty, c);
        let vt = variant_type(self.tt, ty, variant, c);
        if vt != t {
            let p = self.new_out(ty);
            // A shorter variant leaves some of the union's storage untouched.
            self.ins(format!("store {} zeroinitializer, ptr {}", t, p))?;
            self.ins(format!("store {} {}, ptr {}", vt, acc, p))?;
            acc = self.op(format!("load {}, ptr {}", t, p))?;
        }
        Ok(V::agg(acc, ty))
    }

    /// Store the owned value v into the field path of the extended local slot sl (in place). The old value of the last
    /// field is released (nodes: drop then free; buffers: free) after v is complete.
    fn assign_path(&mut self, sl: &Slot, path: &[String], v: &V) -> R<()> {
        let tt = self.tt;
        let mut cur = sl.a.clone();
        let mut cty = need_ty(&sl.ty)?.to_string();
        for (k, fname) in path.iter().enumerate() {
            let i = t_field_index(tt, &cty, fname, None)?;
            let fds = t_fields(tt, &cty, None)?;
            let fd = fds.get(i).ok_or_else(|| IDX_ERR.to_string())?;
            let g = self.op(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 {}", lty(&cty), cur, i))?;
            let boxed = self.fb(&cty, None, fname);
            if k + 1 < path.len() {
                cur = if boxed { self.op(format!("load ptr, ptr {}", g))? } else { g };
                cty = fd.ty.clone();
                continue;
            }
            let ft = fd.ty.as_str();
            if boxed {
                let old = self.op(format!("load ptr, ptr {}", g))?;
                self.ins(format!("call void @pruv_dropnode__{}(ptr {})", ft, old))?;
                let nd = self.op(format!("call ptr @pruv_box__{}({} {})", ft, lty(ft), v.a))?;
                self.ins(format!("store ptr {}, ptr {}", nd, g))?;
            } else if is_arr_name(ft) {
                let x = self.op(format!("load {}, ptr {}", ARR, g))?;
                let old = self.unpack(&x, ft, false)?; // the old buffer is freed with its cap
                self.free_val(&old)?;
                let a = self.agg(v)?;
                self.ins(format!("store {} {}, ptr {}", ARR, a, g))?;
            } else if is_collection(ft) {
                self.drop_slot(&V { ty: Some(ft.to_string()), ..V::new(Kind::Coll, g.clone()) })?;
                self.ins(format!("store ptr {}, ptr {}", v.a, g))?;
            } else if self.is_ext(ft) {
                self.drop_agg_slot(ft, &g)?;
                self.ins(format!("store {} {}, ptr {}", lty(ft), v.a, g))?;
            } else {
                let x = self.pack(v, ft, false)?;
                self.ins(format!("store {} {}, ptr {}", st(ft, false), x, g))?;
            }
        }
        Ok(())
    }

    /// Move a temporary into a fresh owner slot (returned); the caller registers it in a scope.
    fn hold(&mut self, v: &V) -> R<Slot> {
        let sl = if v.k == Kind::Agg || v.k == Kind::Coll {
            let ty = need_ty(&v.ty)?.to_string();
            self.new_slot("hold", &ty)?
        } else {
            let mut sl = self.new_slot("hold", "array")?;
            sl.ty = v.ty.clone();
            sl
        };
        self.store(&sl, v)?;
        Ok(sl)
    }

    // ---------- G6 A3: shim calls, owner slots ----------
    fn g6_decl(&mut self, sym: &str, ret: &str, argtys: &[&str]) {
        self.m.g6_decls.insert(sym.to_string(), format!("declare {} @{}({}) nounwind", ret, sym, argtys.join(", ")));
        self.bridge = true;
    }

    /// Calls a bridge symbol (sym is the unprefixed foreign_shim name; the program prefix is added here);
    /// args: (llvm type, value). Returns the SSA result ("" for void).
    fn g6_call(&mut self, sym: &str, ret: &str, args: &[(String, String)]) -> R<String> {
        let sym_owned = format!("{}{}", self.m.sym_pre, sym);
        let sym = sym_owned.as_str();
        let tys: Vec<&str> = args.iter().map(|(t, _)| t.as_str()).collect();
        self.g6_decl(sym, ret, &tys);
        let a: Vec<String> = args.iter().map(|(t, v)| format!("{} {}", t, v)).collect();
        let a = a.join(", ");
        if ret == "void" {
            self.ins(format!("call void @{}({})", sym, a))?;
            return Ok(String::new());
        }
        self.op(format!("call {} @{}({})", ret, sym, a))
    }

    fn g6_drop(&mut self, v: &V) -> R<()> {
        let ty = need_ty(&v.ty)?.to_string();
        let sym = crate::foreign_shim::drop_sym(&ty)?;
        self.g6_call(&sym, "void", &[("ptr".to_string(), v.a.clone())])?;
        Ok(())
    }

    /// Drops the owner slot's collection if it is non-null (moved-out slots hold null).
    fn drop_slot(&mut self, sl: &Slot) -> R<()> {
        let p = self.op(format!("load ptr, ptr {}", sl.a))?;
        let nn = self.op(format!("icmp ne ptr {}, null", p))?;
        let dl = self.label("gdrop");
        let el = self.label("gdone");
        self.condbr(&nn, &dl, &el)?;
        self.place(&dl);
        let v = V { layout: "i64", rng: None, k: Kind::Coll, a: p, b: "None".to_string(), ty: sl.ty.clone(), p: None, mv: None, c: None };
        self.g6_drop(&v)?;
        self.br(&el)?;
        self.place(&el);
        Ok(())
    }

    pub fn pop_scope(&mut self) -> R<()> {
        let sc = self.scopes.pop().ok_or_else(|| "pop from empty list".to_string())?;
        if !self.dead {
            self.free_scope(&sc.arrays)?;
        }
        Ok(())
    }

    pub fn begin_temps(&mut self) {
        self.temps.push(Vec::new());
    }

    pub fn end_temps(&mut self) -> R<()> {
        let ts = self.temps.pop().ok_or_else(|| "pop from empty list".to_string())?;
        if !self.dead {
            for v in ts.iter().rev() {
                self.free_val(v)?;
            }
        }
        Ok(())
    }

    // ---------- user aggregates (G1) ----------
    /// SSA of the storage type st(ty, c) of value v
    pub fn pack(&mut self, v: &V, ty: &str, c: bool) -> R<String> {
        if ty == "option" || ty == "result" {
            let (ot, pt) = if c { (COPT, "i128") } else { (OPT, "i64") };
            let a = self.op(format!("insertvalue {} undef, i1 {}, 0", ot, v.a))?;
            return self.op(format!("insertvalue {} {}, {} {}, 1", ot, a, pt, v.b));
        }
        if is_arr_name(ty) {
            // G9 A3: a buffer field is stored as { ptr, i64, i64 } (data, len, cap)
            return self.agg(v);
        }
        Ok(v.a.clone())
    }

    pub fn unpack(&mut self, x: &str, ty: &str, c: bool) -> R<V> {
        if is_collection(ty) {
            return Ok(V { ty: Some(ty.to_string()), ..V::new(Kind::Coll, x) });
        }
        if is_arr_name(ty) {
            // G9 A3
            let p0 = self.op(format!("extractvalue {} {}, 0", ARR, x))?;
            let p1 = self.op(format!("extractvalue {} {}, 1", ARR, x))?;
            let mut v = V::with_b(Kind::Arr, p0, p1);
            v.ty = elem_ty(ty);
            v.c = Some(self.op(format!("extractvalue {} {}, 2", ARR, x))?);
            return Ok(v);
        }
        match ty {
            "i64" => Ok(V::new(if c { Kind::I128 } else { Kind::I64 }, x)),
            "bool" => Ok(V::new(Kind::I1, x)),
            "option" | "result" => {
                let ot = if c { COPT } else { OPT };
                let t = self.op(format!("extractvalue {} {}, 0", ot, x))?;
                let p = self.op(format!("extractvalue {} {}, 1", ot, x))?;
                Ok(V::with_b(Kind::Opt, t, p))
            }
            _ => Ok(V::agg(x, ty)),
        }
    }

    pub fn put(&mut self, base: &str, ty: &str, idx: usize, v: &V, fty: &str, c: bool) -> R<String> {
        let x = self.pack(v, fty, c)?;
        self.op(format!("insertvalue {} {}, {} {}, {}", st(ty, c), base, st(fty, c), x, idx))
    }

    pub fn get(&mut self, base: &V, ty: &str, idx: usize, fty: &str, c: bool) -> R<V> {
        let x = self.op(format!("extractvalue {} {}, {}", st(ty, c), base.a, idx))?;
        self.unpack(&x, fty, c)
    }

    pub fn tag_of(&mut self, v: &V, c: bool) -> R<String> {
        let ty = need_ty(&v.ty)?.to_string();
        self.op(format!("extractvalue {} {}, 0", st(&ty, c), v.a))
    }

    /// `own`: G9 A3, the operand is evaluated in an owning position (code form only).
    fn evf(&mut self, e: &Expr, c: bool, own: bool) -> R<V> {
        if c {
            self.cev(e)
        } else {
            self.ev(e, own)
        }
    }

    /// `own`: G9 A3, constructor fields of an extended type are owning positions.
    pub fn mk_struct(&mut self, e: &StructLit, c: bool, own: bool) -> R<V> {
        let tt = self.tt;
        let ty = e.name.as_str();
        // Look up by name like a Python dict (audit: a linear Vec search is quadratic in the field count). For the same name, the later value wins.
        let mut vals: std::collections::HashMap<&str, V> = std::collections::HashMap::new();
        for fi in &e.fields {
            // Evaluate in source order
            let v = self.evf(&fi.value, c, own)?;
            vals.insert(fi.name.as_str(), v);
        }
        let mut acc = "zeroinitializer".to_string();
        for (i, fd) in t_fields(tt, ty, None)?.iter().enumerate() {
            let v = vals
                .get(fd.name.as_str())
                .cloned()
                .ok_or_else(|| format!("'{}'", fd.name))?;
            // G9: a boxed field moves into a new node
            acc = self.putf(&acc, ty, i, &v, fd, None, c)?;
        }
        Ok(V::agg(acc, ty))
    }

    pub fn mk_enum(&mut self, e: &EnumCtor, c: bool, own: bool) -> R<V> {
        let tt = self.tt;
        let ty = e.enum_name.as_str();
        let mut vals: Vec<V> = Vec::new();
        for x in &e.args {
            vals.push(self.evf(x, c, own)?);
        }
        let tag = t_variant_index(tt, ty, &e.variant)?;
        let variant = (!e.args.is_empty()).then_some(e.variant.as_str());
        let t = variant_type(tt, ty, variant, c);
        let mut acc = self.op(format!("insertvalue {} zeroinitializer, i64 {}, 0", t, tag))?;
        for (i, fd) in t_fields(tt, ty, Some(&e.variant))?.iter().enumerate() {
            let v = vals.get(i).ok_or_else(|| IDX_ERR.to_string())?;
            acc = self.putf(&acc, ty, i, v, fd, Some(&e.variant), c)?;
        }
        self.finish_enum(acc, ty, variant, c)
    }

    /// Some/None/Ok/Err of a new Option<X>/Result<X> application (e.ty): zeroinitializer-based, so the unused
    /// payload storage is always zero.
    pub fn mk_gctor(&mut self, e: &Ctor, c: bool) -> R<V> {
        let ty = need_ty(&e.ty)?.to_string();
        if e.name == "None" {
            return Ok(V::agg("zeroinitializer", &ty));
        }
        if e.arg.is_none() && e.name == "Ok" {
            // LB8 R-6g7-1: Ok(()) sets only the tag
            let r = self.op(format!("insertvalue {} zeroinitializer, i1 true, 0", st(&ty, c)))?;
            return Ok(V::agg(r, &ty));
        }
        let a = e.arg.as_ref().ok_or_else(|| "llvmgen: constructor without argument".to_string())?;
        let x = self.evf(a, c, false)?;
        if e.name == "Some" || e.name == "Ok" {
            let acc = self.op(format!("insertvalue {} zeroinitializer, i1 true, 0", st(&ty, c)))?;
            let r = self.put(&acc, &ty, 1, &x, g_arg(&ty), c)?;
            return Ok(V::agg(r, &ty));
        }
        // Err: tag false (zero), error payload in slot 2
        let r = self.put("zeroinitializer", &ty, 2, &x, g_error(&ty), c)?;
        Ok(V::agg(r, &ty))
    }

    /// match on a new Option<X>/Result<X>: branch on the tag, bind the active payload only inside its arm
    pub fn gmatch(&mut self, st_: &Match, sv: &V) -> R<()> {
        let ty = need_ty(&sv.ty)?.to_string();
        let x = g_arg(&ty).to_string();
        let pos = st_
            .arms
            .iter()
            .find(|a| a.ctor == "Some" || a.ctor == "Ok")
            .ok_or_else(|| IDX_ERR.to_string())?;
        let neg = st_
            .arms
            .iter()
            .find(|a| a.ctor == "None" || a.ctor == "Err")
            .ok_or_else(|| IDX_ERR.to_string())?;
        let tag = self.op(format!("extractvalue {} {}, 0", st(&ty, false), sv.a))?;
        let pl = self.label("armT");
        let nl = self.label("armF");
        let end = self.label("endmatch");
        self.condbr(&tag, &pl, &nl)?;
        let mut reach = false;
        for (arm, lbl) in [(pos, &pl), (neg, &nl)] {
            self.place(lbl);
            let mut bind: Vec<(String, Bind)> = Vec::new();
            if let Some(b0) = arm.binders.first() {
                if arm.ctor == "Some" || arm.ctor == "Ok" {
                    let v = self.get(sv, &ty, 1, &x, false)?;
                    bind.push((b0.clone(), Bind::Ssa(v)));
                } else {
                    let v = self.get(sv, &ty, 2, g_error(&ty), false)?;
                    bind.push((b0.clone(), Bind::Ssa(v)));
                }
            }
            if self.run_block(&arm.body, bind)? {
                self.br(&end)?;
                reach = true;
            }
        }
        if reach {
            self.place(&end);
        } else {
            self.dead = true;
        }
        Ok(())
    }

    /// The aggregate `base` (code form) with the nested struct field `path` replaced by v
    pub fn set_path(&mut self, base: &V, ty: &str, path: &[String], v: &V) -> R<V> {
        let tt = self.tt;
        let fds = t_fields(tt, ty, None)?;
        let first = path.first().ok_or_else(|| IDX_ERR.to_string())?;
        let i = t_field_index(tt, ty, first, None)?;
        let fty = fds.get(i).ok_or_else(|| IDX_ERR.to_string())?.ty.as_str();
        let nv = if path.len() == 1 {
            v.clone()
        } else {
            let sub = self.get(base, ty, i, fty, false)?;
            self.set_path(&sub, fty, &path[1..], v)?
        };
        let r = self.put(&base.a, ty, i, &nv, fty, false)?;
        Ok(V::agg(r, ty))
    }

    /// Code value -> contract value (i64 leaves are sign-extended; for an enum only the active variant is converted)
    pub fn to_c(&mut self, x: &V, ty: &str) -> R<V> {
        let tt = self.tt;
        match ty {
            "i64" => {
                let r = self.op(format!("sext i64 {} to i128", x.a))?;
                return Ok(V::new(Kind::I128, r));
            }
            "bool" => return Ok(x.clone()),
            "option" | "result" => {
                let r = self.op(format!("sext i64 {} to i128", x.b))?;
                return Ok(V::with_b(Kind::Opt, x.a.clone(), r));
            }
            _ => {}
        }
        if is_gopt(ty) {
            // G3: widen the payload (and the Result error); the unused payload stays zero
            let xt = g_arg(ty).to_string();
            let t = self.op(format!("extractvalue {} {}, 0", st(ty, false), x.a))?;
            let mut acc = self.op(format!("insertvalue {} zeroinitializer, i1 {}, 0", st(ty, true), t))?;
            if xt != "unit" {
                // LB8: the Ok slot of Result<(), E> is zero-sized
                let px = self.get(x, ty, 1, &xt, false)?;
                let pc = self.to_c(&px, &xt)?;
                acc = self.put(&acc, ty, 1, &pc, &xt, true)?;
            }
            if ty.starts_with("result<") {
                let et = g_error(ty);
                let er = self.get(x, ty, 2, et, false)?;
                let ec = self.to_c(&er, et)?;
                acc = self.put(&acc, ty, 2, &ec, et, true)?;
            }
            return Ok(V::agg(acc, ty));
        }
        let d = t_decl(tt, ty)?;
        match d {
            TypeDecl::Struct(sd) => {
                let mut acc = "zeroinitializer".to_string();
                for (i, f) in sd.fields.iter().enumerate() {
                    let fx = self.get(x, ty, i, &f.ty, false)?;
                    let cx = self.to_c(&fx, &f.ty)?;
                    acc = self.put(&acc, ty, i, &cx, &f.ty, true)?;
                }
                Ok(V::agg(acc, ty))
            }
            TypeDecl::Enum(ed) => {
                let tag = self.tag_of(x, false)?;
                let base = self.op(format!("insertvalue {} zeroinitializer, i64 {}, 0", st(ty, true), tag))?;
                let vs: Vec<&VariantDecl> = ed.variants.iter().filter(|v| !v.fields.is_empty()).collect();
                if vs.is_empty() {
                    return Ok(V::agg(base, ty));
                }
                let lcur = self.cur.clone();
                let join = self.label("cvend");
                let mut lbls: Vec<String> = Vec::new();
                for _ in &vs {
                    lbls.push(self.label("cvv"));
                }
                let mut cases: Vec<String> = Vec::new();
                for (v, l) in vs.iter().zip(&lbls) {
                    cases.push(format!("i64 {}, label %{}", t_variant_index(tt, ty, &v.name)?, l));
                }
                self.term(format!("switch i64 {}, label %{} [ {} ]", tag, join, cases.join(" ")))?;
                let mut entries: Vec<(String, String)> = vec![(base.clone(), lcur)];
                for (v, l) in vs.iter().zip(&lbls) {
                    self.place(l);
                    let mut acc = base.clone();
                    for (i, f) in v.fields.iter().enumerate() {
                        let pi = flattened_payload_index(tt, ty, &v.name, i)?;
                        let fx = if tt.union_words.contains_key(ty) {
                            self.getf(x, ty, i, f, Some(&v.name), false)?
                        } else {
                            self.get(x, ty, pi, &f.ty, false)?
                        };
                        let cx = self.to_c(&fx, &f.ty)?;
                        acc = self.put(&acc, ty, pi, &cx, &f.ty, true)?;
                    }
                    entries.push((acc, self.cur.clone()));
                    self.br(&join)?;
                }
                self.place(&join);
                let ents: Vec<String> = entries.iter().map(|(a, b)| format!("[ {}, %{} ]", a, b)).collect();
                let r = self.op(format!("phi {} {}", st(ty, true), ents.join(", ")))?;
                Ok(V::agg(r, ty))
            }
        }
    }

    fn fields_eq(&mut self, l: &V, r: &V, ty: &str, fds: &[FieldDecl], variant: Option<&str>) -> R<String> {
        let tt = self.tt;
        let mut acc = "true".to_string();
        for (i, f) in fds.iter().enumerate() {
            let j = match variant {
                None => i,
                Some(v) => flattened_payload_index(tt, ty, v, i)?,
            };
            let lx = self.get(l, ty, j, &f.ty, true)?;
            let rx = self.get(r, ty, j, &f.ty, true)?;
            let e = self.ceq(&lx, &rx, &f.ty)?;
            acc = if acc == "true" { e } else { self.op(format!("and i1 {}, {}", acc, e))? };
        }
        Ok(acc)
    }

    /// Structural equality of two contract values -> i1 (SSA name or literal)
    pub fn ceq(&mut self, l: &V, r: &V, ty: &str) -> R<String> {
        let tt = self.tt;
        match ty {
            "i64" => return self.op(format!("icmp eq i128 {}, {}", l.a, r.a)),
            "bool" => return self.op(format!("icmp eq i1 {}, {}", l.a, r.a)),
            "option" | "result" => {
                let te = self.op(format!("icmp eq i1 {}, {}", l.a, r.a))?;
                let mut pe = self.op(format!("icmp eq i128 {}, {}", l.b, r.b))?;
                if ty == "option" {
                    // The payload of None is not compared
                    let nt = self.op(format!("xor i1 {}, true", l.a))?;
                    pe = self.op(format!("or i1 {}, {}", nt, pe))?;
                }
                return self.op(format!("and i1 {}, {}", te, pe));
            }
            _ => {}
        }
        if is_gopt(ty) {
            // G3: only the active payload is compared
            let xt = g_arg(ty).to_string();
            let tc = st(ty, true);
            let tl = self.op(format!("extractvalue {} {}, 0", tc, l.a))?;
            let tr = self.op(format!("extractvalue {} {}, 0", tc, r.a))?;
            let te = self.op(format!("icmp eq i1 {}, {}", tl, tr))?;
            let pe = if xt == "unit" {
                "true".to_string() // LB8
            } else {
                let lx = self.get(l, ty, 1, &xt, true)?;
                let rx = self.get(r, ty, 1, &xt, true)?;
                self.ceq(&lx, &rx, &xt)?
            };
            let nt = self.op(format!("xor i1 {}, true", tl))?;
            if ty.starts_with("option<") {
                let o = self.op(format!("or i1 {}, {}", nt, pe))?;
                return self.op(format!("and i1 {}, {}", te, o));
            }
            let et = g_error(ty);
            let el = self.get(l, ty, 2, et, true)?;
            let er = self.get(r, ty, 2, et, true)?;
            let ee = self.ceq(&el, &er, et)?;
            let a1 = self.op(format!("and i1 {}, {}", tl, pe))?;
            let a2 = self.op(format!("and i1 {}, {}", nt, ee))?;
            let act = self.op(format!("or i1 {}, {}", a1, a2))?;
            return self.op(format!("and i1 {}, {}", te, act));
        }
        let d = t_decl(tt, ty)?;
        match d {
            TypeDecl::Struct(sd) => self.fields_eq(l, r, ty, &sd.fields, None),
            TypeDecl::Enum(ed) => {
                let tl = self.tag_of(l, true)?;
                let tr = self.tag_of(r, true)?;
                let teq = self.op(format!("icmp eq i64 {}, {}", tl, tr))?;
                let vs: Vec<&VariantDecl> = ed.variants.iter().filter(|v| !v.fields.is_empty()).collect();
                if vs.is_empty() {
                    return Ok(teq);
                }
                let lcur = self.cur.clone();
                let same = self.label("ceqs");
                let dflt = self.label("ceqd");
                let end = self.label("ceqe");
                let mut lbls: Vec<String> = Vec::new();
                for _ in &vs {
                    lbls.push(self.label("ceqv"));
                }
                self.condbr(&teq, &same, &end)?;
                let mut entries: Vec<(String, String)> = vec![("false".to_string(), lcur)];
                self.place(&same);
                let mut cases: Vec<String> = Vec::new();
                for (v, lb) in vs.iter().zip(&lbls) {
                    cases.push(format!("i64 {}, label %{}", t_variant_index(tt, ty, &v.name)?, lb));
                }
                self.term(format!("switch i64 {}, label %{} [ {} ]", tl, dflt, cases.join(" ")))?;
                self.place(&dflt); // same tag, variant with no fields
                entries.push(("true".to_string(), dflt.clone()));
                self.br(&end)?;
                for (v, lb) in vs.iter().zip(&lbls) {
                    self.place(lb);
                    let e = self.fields_eq(l, r, ty, &v.fields, Some(&v.name))?;
                    entries.push((e, self.cur.clone()));
                    self.br(&end)?;
                }
                self.place(&end);
                let ents: Vec<String> = entries.iter().map(|(a, b)| format!("[ {}, %{} ]", a, b)).collect();
                self.op(format!("phi i1 {}", ents.join(", ")))
            }
        }
    }

    pub fn agg(&mut self, v: &V) -> R<String> {
        if v.k == Kind::Agg {
            return Ok(v.a.clone());
        }
        if v.k == Kind::Arr {
            let a = self.op(format!("insertvalue {} undef, ptr {}, 0", ARR, v.a))?;
            let a = self.op(format!("insertvalue {} {}, i64 {}, 1", ARR, a, v.b))?;
            return self.op(format!("insertvalue {} {}, i64 {}, 2", ARR, a, capof(v)));
        }
        let a = self.op(format!("insertvalue {} undef, i1 {}, 0", OPT, v.a))?;
        self.op(format!("insertvalue {} {}, i64 {}, 1", OPT, a, v.b))
    }

    // ---------- function skeleton ----------
    pub fn bind_params(&mut self) -> R<()> {
        self.place("start");
        let f = self.f;
        let mut names: HashMap<String, Bind> = HashMap::new();
        let mut owned: Vec<Slot> = Vec::new();
        for (i, p) in f.params.iter().enumerate() {
            let n = mangle(&p.name);
            if p.mode_pos.is_some() && is_array_name(&p.ty) && self.m.resize_params.contains(&(f.name.clone(), i)) {
                let slot = self.array_slot(format!("%p.{n}"), &p.ty)?;
                names.insert(p.name.clone(), Bind::Slot(slot));
            } else if p.mode_pos.is_some() && matches!(p.ty.as_str(), "bool" | "i64") {
                let slot = V::new(vkind(&p.ty), format!("%p.{n}"));
                names.insert(p.name.clone(), Bind::Slot(slot));
            } else if is_arr_name(&p.ty) {
                let mut av = V::with_b(Kind::Arr, format!("%p.{}.a", n), format!("%p.{}.n", n));
                av.layout = self.m.storage.layout_of(p);
                av.rng = self.m.storage.range_of(p);
                if let Some(t) = elem_ty(&p.ty) {
                    av.ty = Some(t); // G2 A5
                }
                if p.take && is_array_name(&p.ty) {
                    av.c = Some(format!("%p.{}.c", n));
                    let sl = self.new_slot(&p.name, &p.ty)?;
                    self.store(&sl, &av)?;
                    names.insert(p.name.clone(), Bind::Slot(sl.clone()));
                    owned.push(sl);
                } else {
                    names.insert(p.name.clone(), Bind::Ssa(av));
                }
            } else if p.ty == "bool" {
                names.insert(p.name.clone(), Bind::Ssa(V::new(Kind::I1, format!("%p.{}", n))));
            } else if p.ty == "i64" {
                names.insert(p.name.clone(), Bind::Ssa(V::new(Kind::I64, format!("%p.{}", n))));
            } else if is_collection(&p.ty) {
                // G6 A3
                self.bridge = true;
                if p.take {
                    // consuming parameter: an owner slot (the callee drops it unless it is moved out)
                    let sl = self.new_slot(&p.name, &p.ty)?;
                    self.ins(format!("store ptr %p.{}.h, ptr {}", n, sl.a))?;
                    names.insert(p.name.clone(), Bind::Slot(sl.clone()));
                    owned.push(sl);
                } else {
                    // borrowed: never dropped here
                    names.insert(
                        p.name.clone(),
                        Bind::Ssa(V {
                            layout: "i64",
                            rng: None,
                            k: Kind::Coll,
                            a: format!("%p.{}.h", n),
                            b: "None".to_string(),
                            ty: Some(p.ty.clone()),
                            p: None,
                            mv: None,
                            c: None,
                        }),
                    );
                }
            } else if p.mode_pos.is_some() && is_agg(&p.ty) {
                names.insert(p.name.clone(), Bind::Slot(V::agg(format!("%p.{}", n), &p.ty)));
            } else if self.is_ext(&p.ty) && p.take {
                // LG9: the callee owns the storage; dropped at exit unless moved out
                let sl = V::agg(format!("%p.{}", n), &p.ty);
                names.insert(p.name.clone(), Bind::Slot(sl.clone()));
                owned.push(sl);
            } else if self.is_ext(&p.ty) {
                // G9 A3: a read view of caller-owned storage; never dropped here
                let pv = self.op(format!("load {}, ptr %p.{}", lty(&p.ty), n))?;
                names.insert(p.name.clone(), Bind::Ssa(V::agg_p(pv, &p.ty, format!("%p.{}", n))));
            } else if is_agg(&p.ty) {
                names.insert(p.name.clone(), Bind::Ssa(V::agg(format!("%p.{}", n), &p.ty)));
            } else {
                let t = self.op(format!("extractvalue {} %p.{}, 0", OPT, n))?;
                let v = self.op(format!("extractvalue {} %p.{}, 1", OPT, n))?;
                names.insert(p.name.clone(), Bind::Ssa(V::with_b(Kind::Opt, t, v)));
            }
        }
        self.scopes.push(Scope { names, arrays: owned });
        Ok(())
    }

    pub fn emit_ret(&mut self, v: &V) -> R<()> {
        if self.cost { self.cost_check()?; }
        let f = self.f;
        let rt = f.ret.as_str();
        if view_ret(f) {
            // R-6g9-1: a borrowed view { data, len }
            let g0 = self.op(format!("insertvalue {VIEW} undef, ptr {}, 0", v.a))?;
            let g1 = self.op(format!("insertvalue {VIEW} {g0}, i64 {}, 1", v.b))?;
            self.term(format!("ret {VIEW} {g1}"))
        } else if self.is_ext(rt) {
            // G9 A3: explicit output storage; v is an owned value
            self.ins(format!("store {} {}, ptr %out", lty(rt), v.a))?;
            self.term("ret void".to_string())
        } else if rt == "unit" {
            self.term("ret void".to_string())
        } else if rt == "i64" || rt == "bool" || is_collection(rt) {
            self.term(format!("ret {} {}", lty(rt), v.a))
        } else {
            let g = self.agg(v)?;
            self.term(format!("ret {} {}", lty(rt), g))
        }
    }

    // ---------- statements ----------
    /// Lowers a block into its own scope; true if control can fall out of it
    pub fn run_block(&mut self, stmts: &[Stmt], bind: Vec<(String, Bind)>) -> R<bool> {
        self.run_block_in(stmts, bind, false)
    }

    pub fn run_block_in(&mut self, stmts: &[Stmt], bind: Vec<(String, Bind)>, lp: bool) -> R<bool> {
        self.run_block_with_owner(stmts, bind, lp, None)
    }

    fn run_block_with_owner(&mut self, stmts: &[Stmt], bind: Vec<(String, Bind)>, lp: bool, own: Option<Slot>) -> R<bool> {
        self.push_scope();
        let decls: HashSet<String> = bind.iter().map(|(k, _)| k.clone()).collect();
        if !bind.is_empty() || own.is_some() {
            let sc = self.scopes.last_mut().ok_or_else(|| IDX_ERR.to_string())?;
            for (k, v) in bind {
                sc.names.insert(k, v);
            }
            sc.arrays.extend(own);
        }
        // later[i]: names any statement from i on names
        let mut later: Vec<HashSet<String>> = vec![HashSet::new(); stmts.len() + 1];
        for i in (0..stmts.len()).rev() {
            let mut n = later[i + 1].clone();
            crate::refine::stmt_names(std::slice::from_ref(&stmts[i]), &mut n);
            later[i] = n;
        }
        self.frames.push((decls, HashSet::new(), lp, false));
        for (i, s) in stmts.iter().enumerate() {
            if let Some(fr) = self.frames.last_mut() {
                fr.1 = std::mem::take(&mut later[i + 1]);
                // LG34: the next statement is a return
                fr.3 = i + 1 < stmts.len() && matches!(stmts[i + 1], Stmt::Return(_));
            }
            self.unbox_next = matches!(s, Stmt::Let(l) if self.unbox_let(l, &stmts[i + 1..]));
            self.tail = match s {
                Stmt::Let(l) => self.tail_fill(l, &stmts[i + 1..]),
                _ => None,
            };
            self.stmt(s)?;
            if let Stmt::Let(l) = s {
                if let Some(fr) = self.frames.last_mut() {
                    fr.0.insert(l.name.clone());
                }
            }
            if self.dead {
                break;
            }
        }
        self.frames.pop();
        self.pop_scope()?;
        Ok(!self.dead)
    }

    /// P3 (the Rust backend's dead_after): the statement being lowered is the last use of local `name`: no later
    /// statement of any enclosing block up to the one declaring it (or binding it, for a match arm) names it, and no
    /// loop body lies between; a parameter is never dead here
    pub fn dead_after(&self, name: &str) -> bool {
        for (decls, later, lp, _) in self.frames.iter().rev() {
            if later.contains(name) {
                return false;
            }
            if decls.contains(name) {
                return true;
            }
            if *lp {
                return false;
            }
        }
        false
    }

    /// LG34 (G-LG33-2, the Rust backend's return_next): the statement after the one being lowered is a return of the
    /// same block that does not name local `name`, so `name` is dead even inside a loop body (control leaves the
    /// function before any later iteration); an owned (take) parameter in scope 0 is never dead here
    pub fn return_next(&self, name: &str) -> bool {
        let Some(fr) = self.frames.last() else { return false };
        if !fr.3 || fr.1.contains(name) {
            return false;
        }
        let Ok(Bind::Slot(v)) = self.lookup(name) else { return false };
        !self.scopes.first().is_some_and(|sc| sc.arrays.iter().any(|y| y.a == v.a))
    }

    /// --no-verify guards of resize(src, cnt, v): len(src) <= cnt <= 2^40, and v <= 255 for [u8]
    /// --no-verify guards of resize(src, cnt, v): len(src) <= cnt <= 2^40 (0 <= cnt for R-6g9-1's scratch resize of
    /// an inout view source, which may shrink), and v <= 255 for [u8]
    fn resize_guards(&mut self, src: &V, cnt: &V, v: &V, scratch: bool) -> R<()> {
        if !self.verified {
            let lo = self.op(format!("icmp sle i64 {}, {}", if scratch { "0" } else { src.b.as_str() }, cnt.a))?;
            let hi = self.op(format!("icmp ule i64 {}, 1099511627776", cnt.a))?;
            let ok = self.op(format!("and i1 {}, {}", lo, hi))?;
            self.guard(&ok, Fail::Panic1("pruv_panic_arith", "resize length outside len..2^40".to_string()), false)?;
            if src.ty.as_deref() == Some(ARRAY_U8) {
                let ok2 = self.op(format!("icmp ule i64 {}, 255", v.a))?;
                self.guard(&ok2, Fail::Elem(v.a.clone(), "u8 0..=255"), false)?;
            }
        }
        Ok(())
    }

    /// Apply a self-assigned resize or truncate to an owned local or inout array slot.
    fn regrow_assign(&mut self, name: &str, value: &Expr, sl: &Slot) -> R<bool> {
        let Expr::Call(c) = value else { return Ok(false) };
        if self.cost || sl.k != Kind::Arr || !matches!(c.name.as_str(), "resize" | "truncate") || self.funcs.contains_key(&c.name) {
            return Ok(false);
        }
        let Expr::Var(x) = &c.args[0] else { return Ok(false) };
        if x.name != name {
            return Ok(false);
        }
        if c.name == "truncate" {
            self.truncate_len(c, &sl.b)?;
            return Ok(true);
        }
        let layout = self.slot_layouts.get(&sl.a).copied().unwrap_or(sl.layout);
        let src = self.load(sl)?; // a = ptr, b = len, c = cap
        let (q, cnt, nc) = self.regrow_val(&src, c)?;
        let mut nv = V::with_b(Kind::Arr, q, cnt.a.clone());
        nv.ty = sl.ty.clone();
        nv.layout = layout;
        nv.rng = self.slot_rngs.get(&sl.a).copied().unwrap_or(sl.rng);
        nv.c = Some(nc);
        self.store(sl, &nv)?;
        Ok(true)
    }

    /// Apply a self-assigned resize or truncate to an owned buffer field.
    fn regrow_field(&mut self, st: &FieldAssign, sl: &Slot) -> R<bool> {
        let Expr::Call(c) = &st.value else { return Ok(false) };
        if self.cost || !matches!(c.name.as_str(), "resize" | "truncate") || self.funcs.contains_key(&c.name) {
            return Ok(false);
        }
        let Some(sty) = sl.ty.clone() else { return Ok(false) };
        if !self.is_ext(&sty) || !matches!(self.tt.decl(&sty), Some(TypeDecl::Struct(_))) {
            return Ok(false);
        }
        if !crate::codegen::same_field_path(&c.args[0], &st.name, &st.fields) {
            return Ok(false);
        }
        let mut cty = sty;
        for fname in &st.fields[..st.fields.len() - 1] {
            if !matches!(self.tt.decl(&cty), Some(TypeDecl::Struct(_))) {
                return Ok(false);
            }
            let i = t_field_index(self.tt, &cty, fname, None)?;
            let fds = t_fields(self.tt, &cty, None)?;
            cty = fds.get(i).ok_or_else(|| IDX_ERR.to_string())?.ty.clone();
        }
        let last = st.fields.last().ok_or_else(|| IDX_ERR.to_string())?;
        if !matches!(self.tt.decl(&cty), Some(TypeDecl::Struct(_))) || self.fb(&cty, None, last) {
            return Ok(false);
        }
        let (gp, fty) = self.field_ptr_ty(sl, &st.fields)?;
        if !is_arr_name(&fty) {
            return Ok(false);
        }
        if c.name == "truncate" {
            let length = self.op(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 1", ARR, gp))?;
            self.truncate_len(c, &length)?;
            return Ok(true);
        }
        let ld = self.op(format!("load {}, ptr {}", ARR, gp))?;
        let src = self.unpack(&ld, &fty, false)?;
        let (q, cnt, nc) = self.regrow_val(&src, c)?;
        let mut nv = V::with_b(Kind::Arr, q, cnt.a.clone());
        nv.ty = src.ty.clone();
        nv.c = Some(nc);
        let g = self.agg(&nv)?;
        self.ins(format!("store {} {}, ptr {}", ARR, g, gp))?;
        Ok(true)
    }

    /// Return the block, length and capacity after an in-place resize.
    fn regrow_val(&mut self, src: &V, c: &Call) -> R<(String, V, String)> {
        let cnt = self.ev(&c.args[1], false)?;
        let v = self.ev(&c.args[2], false)?;
        let scratch = self.f.borrows_inout_buffer(&c.args[0]);
        self.resize_guards(src, &cnt, &v, scratch)?;
        let (q, nc) = self.regrow_block(src, &cnt, &v)?;
        Ok((q, cnt, nc))
    }

    /// the regrow helper call of src's element kind for the evaluated, guarded resize(src, cnt, v); returns (new
    /// block, new cap max(cnt, cap))
    fn regrow_block(&mut self, src: &V, cnt: &V, v: &V) -> R<(String, String)> {
        let cap = capof(src);
        let q = if src.ty.as_deref() == Some(ARRAY_BOOL) {
            let z = self.op(format!("zext i1 {} to i64", v.a))?;
            self.m.regrow_u8 = true;
            self.op(format!("call ptr @pruv_regrow_u8(ptr {}, i64 {}, i64 {}, i64 {}, i64 {})", src.a, cap, src.b, cnt.a, z))?
        } else if src.ty.as_deref() == Some(ARRAY_U8) {
            self.m.regrow_u8 = true;
            self.op(format!("call ptr @pruv_regrow_u8(ptr {}, i64 {}, i64 {}, i64 {}, i64 {})", src.a, cap, src.b, cnt.a, v.a))?
        } else if src.layout != "i64" {
            self.m.regrow_narrow = true;
            self.op(format!(
                "call ptr @pruv_regrow_narrow(ptr {}, i64 {}, i64 {}, i64 {}, i64 {}, i64 {}, i64 {})",
                src.a, cap, src.b, cnt.a, v.a, elem_size(src.layout), alignment(src.layout)
            ))?
        } else {
            self.m.regrow_i64 = true;
            self.op(format!("call ptr @pruv_regrow(ptr {}, i64 {}, i64 {}, i64 {}, i64 {})", src.a, cap, src.b, cnt.a, v.a))?
        };
        let gt = self.op(format!("icmp sgt i64 {}, {}", cnt.a, cap))?;
        let nc = self.op(format!("select i1 {}, i64 {}, i64 {}", gt, cnt.a, cap))?;
        Ok((q, nc))
    }

    fn truncate_guard(&mut self, old_len: &str, new_len: &str) -> R<()> {
        let ok = self.op(format!("icmp ule i64 {new_len}, {old_len}"))?;
        self.guard(&ok, Fail::Panic1("pruv_panic_arith", "truncate length outside 0..len".into()), false)
    }

    fn truncate_len(&mut self, c: &Call, length: &str) -> R<()> {
        let cnt = self.ev(&c.args[1], false)?;
        if !self.verified {
            let old_len = self.op(format!("load i64, ptr {length}"))?;
            self.truncate_guard(&old_len, &cnt.a)?;
        }
        self.ins(format!("store i64 {}, ptr {length}", cnt.a))
    }

    fn array_grow_stmt(&mut self, c: &Call) -> R<V> {
        let Expr::InoutArg(dest) = &c.args[0] else { return Err("missing inout place".into()); };
        let slot = if let Expr::Var(var) = dest.expr.as_ref() {
            let Bind::Slot(slot) = self.lookup(&var.name)? else { return Err("array needs owned storage".into()); };
            slot
        } else {
            let ptr = self.inout_place_ptr(&dest.expr)?;
            self.array_slot(ptr, need_ty(&dest.ty)?)?
        };
        let src = self.load(&slot)?;
        let value = self.ev(&c.args[1], false)?;
        let push = c.name == "arr_push";
        // LB6 SX (R-6g5-5): arr_push stores k values in order
        let mut values = Vec::new();
        if push {
            values.push(value.a.clone());
            for x in &c.args[2..] {
                values.push(self.ev(x, false)?.a);
            }
        }
        let append = if c.name == "arr_append" {
            Some((self.ev(&c.args[2], false)?.a, self.ev(&c.args[3], false)?.a))
        } else {
            None
        };
        if !self.verified {
            if let Some((start, count)) = &append {
                let remaining = self.op(format!("sub i64 {}, {count}", value.b))?;
                let start_ok = self.op(format!("icmp sge i64 {start}, 0"))?;
                let count_ok = self.op(format!("icmp sge i64 {count}, 0"))?;
                let end_ok = self.op(format!("icmp sle i64 {start}, {remaining}"))?;
                let ok = self.op(format!("and i1 {start_ok}, {count_ok}"))?;
                let ok = self.op(format!("and i1 {ok}, {end_ok}"))?;
                self.guard(&ok, Fail::Oob(start.clone(), value.b.clone()), false)?;
                let remaining = self.op(format!("sub i64 1099511627776, {}", src.b))?;
                let ok = self.op(format!("icmp sle i64 {count}, {remaining}"))?;
                self.guard(&ok, Fail::Panic1("pruv_panic_arith", "arr_append length exceeds 2^40".into()), false)?;
            } else if push {
                let ok = self.op(format!("icmp ule i64 {}, {}", src.b, 1099511627776i64 - values.len() as i64))?;
                self.guard(&ok, Fail::Panic1("pruv_panic_arith", "arr_push length exceeds 2^40".into()), false)?;
                if is_u8_ty(&src.ty) {
                    for v in values.clone() {
                        let ok = self.op(format!("icmp ule i64 {v}, 255"))?;
                        self.guard(&ok, Fail::Elem(v, "u8 0..=255"), false)?;
                    }
                }
            } else {
                let ok = self.op(format!("icmp ule i64 {}, 1099511627776", value.a))?;
                self.guard(&ok, Fail::Panic1("pruv_panic_arith", "arr_reserve length outside 0..2^40".into()), false)?;
            }
        }
        let pushed = values.len().to_string();
        let added = if push { &pushed } else if let Some((_, count)) = &append { count } else { &value.a };
        let need = self.op(format!("add i64 {}, {added}", src.b))?;
        let cap = capof(&src);
        let grow = self.op(format!("icmp ult i64 {cap}, {need}"))?;
        let before = self.cur.clone();
        let regrow = self.label("regrow");
        let done = self.label("grown");
        self.condbr(&grow, &regrow, &done)?;
        self.place(&regrow);
        let newcap = if push || append.is_some() {
            let double = self.op(format!("mul i64 {cap}, 2"))?;
            let enough = self.op(format!("icmp uge i64 {double}, {need}"))?;
            let target = self.op(format!("select i1 {enough}, i64 {double}, i64 {need}"))?;
            let enough = self.op(format!("icmp uge i64 {target}, 4"))?;
            self.op(format!("select i1 {enough}, i64 {target}, i64 4"))?
        } else {
            need.clone()
        };
        let ty = elem_ir(&src);
        let size = if is_bytes_ty(&src.ty) { 1 } else { elem_size(src.layout) };
        self.m.raw_regrow = true;
        let newptr = self.op(format!("call ptr @pruv_regrow_v2(ptr {}, i64 {cap}, i64 {newcap}, i64 {size}, i64 {size})", src.a))?;
        self.ins(format!("store ptr {newptr}, ptr {}", slot.a))?;
        self.ins(format!("store i64 {newcap}, ptr {}", need_c(&slot.c)?))?;
        self.br(&done)?;
        self.place(&done);
        if push || append.is_some() {
            let ptr = self.op(format!("phi ptr [ {}, %{before} ], [ {newptr}, %{regrow} ]", src.a))?;
            let dst = self.op(format!("getelementptr inbounds {ty}, ptr {ptr}, i64 {}", src.b))?;
            if let Some((start, count)) = append {
                let src_ty = elem_ir(&value);
                let inp = self.op(format!("getelementptr inbounds {src_ty}, ptr {}, i64 {start}", value.a))?;
                if ty == src_ty {
                    let size_bytes = if size == 1 { count } else { self.op(format!("mul nuw nsw i64 {count}, {size}"))? };
                    self.ins(format!("call void @llvm.memcpy.p0.p0.i64(ptr {dst}, ptr {inp}, i64 {size_bytes}, i1 false)"))?;
                } else {
                    self.append_widen(&dst, &inp, &count)?;
                }
            } else {
                for (i, v) in values.into_iter().enumerate() {
                    let elem = if src.ty.as_deref() == Some(ARRAY_BOOL) {
                        self.op(format!("zext i1 {v} to i8"))?
                    } else if ty != "i64" {
                        self.op(format!("trunc i64 {v} to {ty}"))?
                    } else {
                        v
                    };
                    let at = if i == 0 { dst.clone() } else { self.op(format!("getelementptr inbounds {ty}, ptr {dst}, i64 {i}"))? };
                    self.ins(format!("store {ty} {elem}, ptr {at}"))?;
                }
            }
            self.ins(format!("store i64 {need}, ptr {}", slot.b))?;
        }
        Ok(V::unit())
    }

    fn append_widen(&mut self, dst: &str, src: &str, count: &str) -> R<()> {
        let pre = self.cur.clone();
        let lp = self.label("append.loop");
        let done = self.label("append.end");
        let empty = self.op(format!("icmp eq i64 {count}, 0"))?;
        self.condbr(&empty, &done, &lp)?;
        self.place(&lp);
        let i = self.tmp();
        let next = self.tmp();
        self.ins(format!("{i} = phi i64 [ 0, %{pre} ], [ {next}, %{lp} ]"))?;
        let inp = self.op(format!("getelementptr inbounds i8, ptr {src}, i64 {i}"))?;
        let out = self.op(format!("getelementptr inbounds i64, ptr {dst}, i64 {i}"))?;
        let byte = self.op(format!("load i8, ptr {inp}"))?;
        let value = self.op(format!("zext i8 {byte} to i64"))?;
        self.ins(format!("store i64 {value}, ptr {out}"))?;
        self.ins(format!("{next} = add nuw nsw i64 {i}, 1"))?;
        let end = self.op(format!("icmp eq i64 {next}, {count}"))?;
        self.condbr(&end, &done, &lp)?;
        self.place(&done);
        Ok(())
    }

    /// LG37 (G-DB56-4, G-DB49-2): the owned local arrays a `str_from_utf8(x)` or `resize(x, n, v)` anywhere in the
    /// expression e of a let / assignment / expression statement / return takes at their last use: x is named once
    /// in the statement (so nothing reads it after the take), is not the assigned target, and is dead after the
    /// statement (dead_after, return_next, or the statement is a return).  x is an owned array slot (a local, or a take
    /// parameter the function owns at a return; not a borrowed or inout parameter, reference, pinned or stack local); for str_from_utf8 also an array payload binder of a consuming match (as
    /// move_src).  The str_from_utf8 lowering then hands x's block and cap to the str, the resize lowering grows x's
    /// block in place (realloc) instead of a fresh block plus a prefix copy.
    fn nested_moves(&self, e: &Expr, target: Option<&str>, ret: bool) -> HashSet<String> {
        self.nested_moves_all(&[e], target, ret)
    }

    /// LG37: nested_moves over several operands of one statement (an element store's index and value), the names
    /// counted across all of them
    fn nested_moves_all(&self, es: &[&Expr], target: Option<&str>, ret: bool) -> HashSet<String> {
        let mut out = HashSet::new();
        if !self.may_take() {
            return out;
        }
        fn walk<'a>(x: &'a Expr, counts: &mut HashMap<String, usize>, calls: &mut Vec<&'a Call>) {
            match x {
                Expr::Var(v) => *counts.entry(v.name.clone()).or_insert(0) += 1,
                Expr::Move(v) => *counts.entry(v.name.clone()).or_insert(0) += 1,
                Expr::Call(c) if matches!(c.args.first(), Some(Expr::Var(_)))
                    && ((c.name == "str_from_utf8" && c.args.len() == 1) || (c.name == "resize" && c.args.len() == 3)) => {
                    calls.push(c)
                }
                _ => {}
            }
            for k in crate::refine::kids(x) {
                walk(k, counts, calls);
            }
        }
        let mut counts: HashMap<String, usize> = HashMap::new();
        let mut calls: Vec<&Call> = Vec::new();
        for e in es {
            walk(e, &mut counts, &mut calls);
        }
        for c in calls {
            let Some(Expr::Var(x)) = c.args.first() else { continue };
            let n = &x.name;
            if Some(n.as_str()) == target || counts.get(n) != Some(&1) {
                continue;
            }
            match self.lookup(n) {
                Ok(Bind::Ssa(v)) => {
                    if c.name == "str_from_utf8" && v.k == Kind::Arr && v.mv.as_ref().is_some_and(|m| m.3) {
                        let nxt = self.frames.last().is_some_and(|fr| fr.3 && !fr.1.contains(n));
                        if ret || nxt || self.dead_after(n) {
                            out.insert(n.clone());
                        }
                    }
                }
                Ok(Bind::Slot(v)) => {
                    if (ret || self.dead_after(n) || self.return_next(n)) && v.k == Kind::Arr && self.owned_slot(&v) {
                        out.insert(n.clone());
                    }
                }
                Err(_) => {}
            }
        }
        out
    }

    /// Every last-use take (move_src, take_src, nested_moves, ret_takable) is on: not a cost build, and R-6g9-1 no
    /// `ref` local in the function, since a live view may read the array (local or field) a take would move, shrink,
    /// regrow or free
    fn may_take(&self) -> bool {
        !self.cost && !self.views
    }

    /// P3 (the Rust backend's last_copy): a let / assignment `y = copy(x)` at the last use of x moves x when x is an
    /// owned local array, or an array payload binder of a match on an owned extended local that is dead after the
    /// match; returns {x} (the copy lowering then takes the value and empties its owner), else an empty set
    pub fn move_src(&self, name: &str, e: &Expr) -> HashSet<String> {
        let mut out = HashSet::new();
        if !self.may_take() {
            return out;
        }
        let Expr::Call(c) = e else { return out };
        if (c.name == "truncate" && c.args.len() == 2) || (c.name == "str_from_utf8" && c.args.len() == 1) {
            // LG33: truncate(x, m) / str_from_utf8(x) of an owned local array at its last use moves x
            let Expr::Var(x) = &c.args[0] else { return out };
            let mut m_names: HashSet<String> = HashSet::new();
            if c.name == "truncate" {
                crate::refine::expr_names(&c.args[1], &mut m_names);
            }
            if x.name == name || m_names.contains(&x.name) {
                return out;
            }
            if let Ok(Bind::Ssa(v)) = self.lookup(&x.name) {
                // LG35 (G-LG34-1): the array payload binder of a match arm on an owned extended local (the Rust
                // backend's consuming match binds the payload Vec by value): at its last use the binder takes the
                // payload block and the owner's field is emptied, so the enum cleanup frees null. The owner must be
                // dead after the match (own_dead: the Rust backend then matches by value, `match p`), and the binder
                // dead after the statement or the next statement a return that does not name it (its last_copy)
                if v.k == Kind::Arr && v.mv.as_ref().is_some_and(|m| m.3) {
                    let nxt = self.frames.last().is_some_and(|fr| fr.3 && !fr.1.contains(&x.name));
                    if nxt || self.dead_after(&x.name) {
                        out.insert(x.name.clone());
                    }
                }
                return out;
            }
            if !(self.dead_after(&x.name) || self.return_next(&x.name)) {
                return out;
            }
            if let Ok(Bind::Slot(v)) = self.lookup(&x.name) {
                if v.k == Kind::Arr && self.owned_slot(&v) {
                    out.insert(x.name.clone());
                }
            }
            return out;
        }
        if c.name != "copy" || c.args.len() != 1 {
            return out;
        }
        if let Expr::FieldAccess(fa) = &c.args[0] {
            // copy(v.f) of an unboxed array field of an owned extended struct local v at its last use moves the
            // field out of v (the Rust backend's last_copy); v's cleanup then frees the emptied field as null
            let Expr::Var(b) = &*fa.base else { return out };
            if b.name == name || !self.dead_after(&b.name) {
                return out;
            }
            if let Ok(Bind::Slot(v)) = self.lookup(&b.name) {
                let Some(ty) = v.ty.as_deref() else { return out };
                if v.k == Kind::Agg && self.is_ext(ty) && self.owned_slot(&v)
                    && matches!(t_decl(self.tt, ty), Ok(TypeDecl::Struct(_)))
                {
                    let fd = t_field_index(self.tt, ty, &fa.field, None)
                        .and_then(|i| t_fields(self.tt, ty, None).map(|fs| fs.get(i).cloned()));
                    if let Ok(Some(fd)) = fd {
                        if is_arr_name(&fd.ty) && !self.fb(ty, None, &fd.name) {
                            out.insert(b.name.clone());
                        }
                    }
                }
            }
            return out;
        }
        let Expr::Var(x) = &c.args[0] else { return out };
        if x.name == name || !self.dead_after(&x.name) {
            return out;
        }
        match self.lookup(&x.name) {
            Ok(Bind::Slot(v)) if v.k == Kind::Arr && self.owned_slot(&v) => {
                out.insert(x.name.clone());
            }
            Ok(Bind::Ssa(v)) if v.k == Kind::Arr && v.mv.as_ref().is_some_and(|m| m.3) => {
                out.insert(x.name.clone());
            }
            _ => {}
        }
        out
    }

    /// P3: empty the owner of array local `name` that a move_src lowering takes: its slot, or for a match payload
    /// binder (V.mv) the owner enum's payload field, so the owner's cleanup frees null
    fn take_array(&mut self, name: &str) -> R<()> {
        match self.lookup(name)? {
            Bind::Slot(x) => self.empty_array_slot(&x),
            Bind::Ssa(x) => {
                let (sl, vn, k, _) = (*x.mv.clone().ok_or_else(|| IDX_ERR.to_string())?).clone();
                let g = g9_field_gep(self, need_ty(&sl.ty)?, &sl.a, Some(&vn), k)?;
                self.ins(format!("store {} zeroinitializer, ptr {}", ARR, g))
            }
        }
    }

    fn empty_array_slot(&mut self, x: &V) -> R<()> {
        self.ins(format!("store ptr null, ptr {}", x.a))?;
        self.ins(format!("store i64 0, ptr {}", x.b))?;
        self.ins(format!("store i64 0, ptr {}", need_c(&x.c)?))
    }

    /// P3: the value of copy(x) when x moves instead (a return candidate or a move_src name), else None
    pub fn copy_move(&mut self, e: &Expr) -> R<Option<V>> {
        if let Expr::FieldAccess(fa) = e {
            if let Expr::Var(b) = &*fa.base {
                if self.moving.contains(&b.name) {
                    // move_src: an owned struct slot; the field moves out and is emptied
                    let Bind::Slot(sl) = self.lookup(&b.name)? else { return Err(IDX_ERR.to_string()) };
                    let ty = need_ty(&sl.ty)?.to_string();
                    let tt = self.tt;
                    let i = t_field_index(tt, &ty, &fa.field, None)?;
                    let ft = t_fields(tt, &ty, None)?.get(i).ok_or_else(|| IDX_ERR.to_string())?.ty.clone();
                    let gp = g9_gep(self, &lty(&ty), &sl.a, i)?;
                    let x = self.op(format!("load {}, ptr {}", ARR, gp))?;
                    self.ins(format!("store {} zeroinitializer, ptr {}", ARR, gp))?;
                    return Ok(Some(self.unpack(&x, &ft, false)?));
                }
            }
        }
        let Expr::Var(n) = e else { return Ok(None) };
        let rc = self.ret_cand.contains(&n.name);
        if !rc && !self.moving.contains(&n.name) {
            return Ok(None);
        }
        match self.lookup(&n.name)? {
            Bind::Slot(x) => {
                let v = self.load(&x)?;
                if rc {
                    self.ret_moved.insert(x.a.clone(), None); // the return's cleanup skips the slot
                } else {
                    self.empty_array_slot(&x)?;
                }
                Ok(Some(v))
            }
            Bind::Ssa(x) => {
                let (sl, vn, k, _) = (*x.mv.clone().ok_or_else(|| IDX_ERR.to_string())?).clone();
                if rc {
                    let mut ks = match self.ret_moved.get(&sl.a) {
                        Some(Some((_, p))) => p.clone(),
                        _ => HashSet::new(),
                    };
                    ks.insert(k);
                    self.ret_moved.insert(sl.a.clone(), Some((vn, ks)));
                } else {
                    let g = g9_field_gep(self, need_ty(&sl.ty)?, &sl.a, Some(&vn), k)?;
                    self.ins(format!("store {} zeroinitializer, ptr {}", ARR, g))?;
                }
                Ok(Some(x))
            }
        }
    }

    pub fn body(&mut self) -> R<()> {
        self.bind_params()?;
        let f = self.f;
        if self.cost {
            let args = ir_params(f, self.tt, &self.m.resize_params).iter().filter(|(_, n)| n != "%out")
                .map(|(t, n)| format!("{} {}", t, n)).collect::<Vec<_>>().join(", ");
            self.cost_bound = self.op(format!("call i128 @{}__pruv_cost({})", mangle(&f.name), args))?;
            self.allocas.push("%cost.spent = alloca i128".into());
            self.ins("store i128 0, ptr %cost.spent".into())?;
        }
        let falls = self.run_block(&f.body, Vec::new())?;
        if falls {
            if f.ret == "unit" {
                // G6 A3: owned (take) parameters; empty for every other program
                let arrays = self.scopes.first().map(|s| s.arrays.clone()).ok_or_else(|| IDX_ERR.to_string())?;
                self.free_scope(&arrays)?;
                self.emit_ret(&V::unit())?;
            } else {
                self.term("unreachable".to_string())?;
            }
        }
        Ok(())
    }

    /// P3-S1: `let x: [E] = alloc(C, v)` on the stack: an entry-block alloca of C elements,
    /// filled every time the declaration runs: @llvm.memset for byte elements (size 1) and literal-zero fills,
    /// pruv_fill_v2 otherwise, and nothing for length 0; the slot is not owned, so nothing is ever freed.
    fn stack_let(&mut self, x: &Let) -> R<()> {
        let n = self.m.stack.length(x);
        self.begin_temps();
        // n is a literal (no code); the fill value is evaluated here, once
        let fill = match &x.value {
            Expr::Call(c) => &c.args[1],
            _ => return Err("llvmgen: stack array without alloc".to_string()),
        };
        self.moving = self.nested_moves(&x.value, Some(&x.name), false); // LG37: a last-use take inside the fill value
        let v = self.ev(fill, false);
        self.moving.clear();
        let v = v?;
        let val: String;
        let layout: &'static str;
        let mut rng: Option<(i128, i128)> = None;
        let size: usize;
        let ety: String;
        if x.ty == "array_bool" {
            val = v.a.clone(); // i1; widened to the fill byte below (no code at all when n == 0)
            layout = "i64"; size = 1; ety = "i8".to_string();
        } else if x.ty == "array_u8" {
            if !self.verified {
                let ok = self.op(format!("icmp ule i64 {}, 255", v.a))?;
                self.guard(&ok, Fail::Elem(v.a.clone(), "u8 0..=255"), false)?;
            }
            val = v.a.clone();
            layout = "i64"; size = 1; ety = "i8".to_string();
        } else {
            layout = self.m.storage.layout_of(x);
            rng = self.m.storage.range_of(x);
            size = elem_size(layout);
            val = v.a.clone();
            ety = format!("i{}", size * 8);
        }
        let sl = self.new_slot(&x.name, &x.ty)?;
        let buf = format!("{}.stk", &sl.a[..sl.a.len() - 2]);
        self.allocas.push(format!("{} = alloca [{} x {}], align {}", buf, n, ety, size));
        if n == 0 {
            // nothing to fill
        } else if size == 1 {
            let byte = if x.ty == "array_bool" {
                self.op(format!("zext i1 {} to i8", val))?
            } else {
                self.op(format!("trunc i64 {} to i8", val))?
            };
            self.ins(format!("call void @llvm.memset.p0.i64(ptr align 1 {}, i8 {}, i64 {}, i1 false)", buf, byte, n))?;
        } else if val == "0" {
            self.ins(format!("call void @llvm.memset.p0.i64(ptr align {} {}, i8 0, i64 {}, i1 false)", size, buf, n * size))?;
        } else {
            self.m.fill = true;
            self.ins(format!("call void @pruv_fill_v2(ptr {}, i64 {}, i64 {}, i64 {})", buf, n, val, size))?;
        }
        let mut av = V::with_b(Kind::Arr, buf, n.to_string());
        av.ty = elem_ty(&x.ty);
        av.layout = layout;
        av.rng = rng;
        self.store(&sl, &av)?;
        let sc = self.scopes.last_mut().ok_or_else(|| IDX_ERR.to_string())?;
        sc.names.insert(x.name.clone(), Bind::Slot(sl));
        self.end_temps()
    }

    fn place_ptr(&mut self, place: &Expr) -> R<String> {
        if let Expr::Call(c) = place {
            if crate::ast::PlaceIf::matches(c) {
                let cond = self.ev(&c.args[0], false)?;
                let then = self.label("pthen");
                let els = self.label("pelse");
                let end = self.label("pend");
                self.condbr(&cond.a, &then, &els)?;
                self.place(&then);
                let a = self.place_ptr(&c.args[1])?;
                let pa = self.cur.clone();
                self.br(&end)?;
                self.place(&els);
                let b = self.place_ptr(&c.args[2])?;
                let pb = self.cur.clone();
                self.br(&end)?;
                self.place(&end);
                return self.op(format!("phi ptr [ {a}, %{pa} ], [ {b}, %{pb} ]"));
            }
        }
        let base = if let Expr::Index(i) = place { i.array.as_ref() } else { place };
        let (root, path) = crate::check::field_place(base);
        let Expr::Var(root) = root else { return Err("invalid place root".into()) };
        let binding = self.lookup(&root.name)?;
        if let Expr::Index(i) = place {
            let arr = match binding {
                Bind::Ssa(v) if path.is_empty() => v,
                Bind::Slot(slot) if path.is_empty() => self.load(&slot)?,
                Bind::Slot(slot) => {
                    let (ptr, ty) = self.field_ptr_ty(&slot, &path)?;
                    let data = self.op(format!("load ptr, ptr {ptr}"))?;
                    let len_ptr = self.op(format!("getelementptr inbounds {ARR}, ptr {ptr}, i32 0, i32 1"))?;
                    let len = self.op(format!("load i64, ptr {len_ptr}"))?;
                    let mut arr = V::with_b(Kind::Arr, data, len);
                    arr.ty = elem_ty(&ty);
                    arr
                }
                _ => return Err(format!("llvmgen: place binding of non-slot {}", root.name)),
            };
            if arr.layout != "i64" {
                return Err(format!("llvmgen: place binding into a narrowed array {}", root.name));
            }
            let index = self.ev(&i.index, false)?;
            return self.index_ptr(&arr, &index);
        }
        let Bind::Slot(slot) = binding else {
            return Err(format!("llvmgen: place binding of non-slot {}", root.name));
        };
        if path.is_empty() { Ok(slot.a) } else { Ok(self.field_ptr_ty(&slot, &path)?.0) }
    }

    pub fn stmt(&mut self, st: &Stmt) -> R<()> {
        match st {
            Stmt::Let(x) if x.borrowed && is_arr_name(&x.ty) => {
                // R-6g9-1: a `ref [u8]` / `ref str` local is a view: no copy, never freed
                self.begin_temps();
                let v = self.ev(&x.value, false)?;
                let sl = self.view_slot(&x.name, &x.ty);
                self.store(&sl, &v)?;
                let sc = self.scopes.last_mut().ok_or_else(|| IDX_ERR.to_string())?;
                sc.names.insert(x.name.clone(), Bind::Slot(sl));
                self.end_temps()
            }
            Stmt::Let(x) if x.is_place() => {
                let Expr::InoutArg(arg) = &x.value else { unreachable!() };
                let kind = vkind(&x.ty);
                if !matches!(kind, Kind::I64 | Kind::I1 | Kind::Agg) || self.is_ext(&x.ty) {
                    let ty = crate::gtype::GType::from_key(&x.ty);
                    return Err(e2_diag(x.line, x.col,
                        &format!("a place binding of type {} needs the Rust backend", ty.name)));
                }
                self.begin_temps();
                let ptr = self.place_ptr(&arg.expr)?;
                self.end_temps()?;
                let mut slot = Slot::new(kind, ptr);
                slot.ty = Some(x.ty.clone());
                self.scopes.last_mut().ok_or_else(|| IDX_ERR.to_string())?
                    .names.insert(x.name.clone(), Bind::Slot(slot));
                Ok(())
            }
            Stmt::Let(x) if self.m.stack.contains(x) && !self.resize_slots.contains(&x.name) => self.stack_let(x),
            Stmt::Let(x) if self.m.stack.lit(x) && !self.cost => {
                // P3: a borrowed literal local binds the read-only global directly: no copy, never freed
                self.begin_temps();
                let v = self.ev(&x.value, false)?;
                let sl = self.new_slot(&x.name, &x.ty)?;
                self.store(&sl, &v)?;
                let sc = self.scopes.last_mut().ok_or_else(|| IDX_ERR.to_string())?;
                sc.names.insert(x.name.clone(), Bind::Slot(sl));
                self.end_temps()
            }
            Stmt::Let(x) if self.unbox_next => {
                // P3 M5: an unboxed String local: the adapter fills an owned str array (null, 0 when empty), no box
                self.unbox_next = false;
                self.begin_temps();
                let Expr::Call(c) = &x.value else { return Err(IDX_ERR.to_string()) };
                let f = self.funcs[&c.name];
                let mut args: Vec<(String, String)> = Vec::new();
                for a in &c.args {
                    let v = self.ev(a, false)?;
                    args.extend(self.g6_arg(&v)?);
                }
                let d = self.g6_out("ptr");
                let l = self.g6_out("i64");
                self.m.g6_unbox.insert(f.name.clone());
                self.m.g6_v2 = true;
                args.push(("ptr".to_string(), d.clone()));
                args.push(("ptr".to_string(), l.clone()));
                self.g6_call(&crate::foreign_shim::unbox_adapter_sym(&f.name), "void", &args)?;
                let dp = self.op(format!("load ptr, ptr {}", d))?;
                let ln = self.op(format!("load i64, ptr {}", l))?;
                let sl = self.new_slot(&x.name, STR)?;
                let mut rv = V::with_b(Kind::Arr, dp, ln);
                rv.ty = Some(STR.to_string());
                self.store(&sl, &rv)?;
                let sc = self.scopes.last_mut().ok_or_else(|| IDX_ERR.to_string())?;
                sc.names.insert(x.name.clone(), Bind::Slot(sl.clone()));
                sc.arrays.push(sl);
                self.end_temps()
            }
            Stmt::Let(x) => {
                self.begin_temps();
                self.moving = self.move_src(&x.name, &x.value);
                self.moving.extend(self.nested_moves(&x.value, Some(&x.name), false));
                self.take_let = self.take_src(&x.name, &x.value);
                let v = self.ev(&x.value, true);
                self.moving.clear();
                self.take_let.clear();
                let v = v?;
                let sl = self.new_slot(&x.name, &x.ty)?;
                self.store(&sl, &v)?;
                // an owner released when the scope ends (G9 A3: also an extended local)
                let owner = sl.k == Kind::Arr || sl.k == Kind::Coll || (sl.k == Kind::Agg && self.is_ext_opt(&sl.ty));
                let sc = self.scopes.last_mut().ok_or_else(|| IDX_ERR.to_string())?;
                sc.names.insert(x.name.clone(), Bind::Slot(sl.clone()));
                if owner {
                    sc.arrays.push(sl);
                }
                self.end_temps()
            }
            Stmt::Assign(x) => {
                self.begin_temps();
                let sl = self.lookup(&x.name)?.val().clone();
                if sl.k == Kind::Arr && sl.c.is_none() {
                    // R-6g9-1: rebinding a view slot: nothing to free
                    let v = self.ev(&x.value, false)?;
                    self.store(&sl, &v)?;
                    return self.end_temps();
                }
                // LG32 (G-DB52-4): x = resize(x, n, v) grows the slot's block in place (realloc), so the old and new blocks are never live together
                // LG37: a last-use take also inside the in-place regrow's operands
                self.moving = self.nested_moves(&x.value, Some(&x.name), false);
                let rg = self.regrow_assign(&x.name, &x.value, &sl);
                if rg.is_err() || rg == Ok(true) {
                    self.moving.clear();
                    rg?;
                    return self.end_temps();
                }
                // the right side is complete and owned before the old value is destroyed
                let ms = self.move_src(&x.name, &x.value);
                self.moving.extend(ms);
                self.take_let = self.take_src(&x.name, &x.value);
                let v = self.ev(&x.value, true);
                self.moving.clear();
                self.take_let.clear();
                let v = v?;
                if sl.k == Kind::Arr {
                    let old = self.load(&sl)?;
                    self.free_val(&old)?;
                } else if sl.k == Kind::Coll {
                    // G6 A3: drop the old value (null when moved out) after the right side
                    self.drop_slot(&sl)?;
                } else if sl.k == Kind::Agg && self.is_ext_opt(&sl.ty) {
                    // G9 A3
                    self.drop_agg_slot(need_ty(&sl.ty)?, &sl.a)?;
                }
                self.store(&sl, &v)?;
                self.end_temps()
            }
            Stmt::FieldAssign(x) => {
                self.begin_temps();
                let sl = match self.lookup(&x.name)? {
                    Bind::Slot(s) => s,
                    Bind::Ssa(_) => return Err(format!("llvmgen: field assignment to non-local {}", x.name)),
                };
                // LG37: a last-use take anywhere in the right side
                self.moving = self.nested_moves(&x.value, Some(&x.name), false);
                let rg = self.regrow_field(x, &sl);
                if rg.is_err() || rg == Ok(true) {
                    // LG34 (G-LG32-3): x.f = resize(x.f, n, v) grows the field in place
                    self.moving.clear();
                    rg?;
                    return self.end_temps();
                }
                if self.is_ext_opt(&sl.ty) {
                    // G9 A3: owned right side first, then an in-place store along the path
                    let v = self.ev(&x.value, true);
                    self.moving.clear();
                    let v = v?;
                    self.assign_path(&sl, &x.fields, &v)?;
                    return self.end_temps();
                }
                let v = self.ev(&x.value, false);
                self.moving.clear();
                let v = v?;
                let ty = need_ty(&sl.ty)?.to_string();
                if matches!(self.tt.decl(&ty), Some(TypeDecl::Struct(_))) {
                    let (gp, fty) = self.field_ptr_ty(&sl, &x.fields)?;
                    let value = self.pack(&v, &fty, false)?;
                    self.ins(format!("store {} {}, ptr {}", self::st(&fty, false), value, gp))?;
                    return self.end_temps();
                }
                let root = self.load(&sl)?;
                let nv = self.set_path(&root, &ty, &x.fields, &v)?;
                self.store(&sl, &nv)?;
                self.end_temps()
            }
            Stmt::IndexAssign(x) => {
                self.begin_temps();
                let bind = self.lookup(&x.name)?;
                // LG37: a last-use take in either operand, the names counted across both
                self.moving = self.nested_moves_all(&[&x.index, &x.value], Some(&x.name), false);
                let i = self.ev(&x.index, false); // index first, then the value (all modes)
                let v = match &i {
                    Ok(_) => self.ev(&x.value, false),
                    Err(e) => Err(e.clone()),
                };
                self.moving.clear();
                let (i, v) = (i?, v?);
                // G5 A3: an inout parameter is a pointer view (ssa), stored through
                let a = if !x.fields.is_empty() {
                    // LG4: `p.a.b[i] = e`: address of the buffer field along the path from the root slot (a local slot or
                    // the pointer of an inout struct parameter); the data pointer and length are read from it
                    let Bind::Slot(sl) = bind else {
                        return Err(format!("llvmgen: element store through non-local {}", x.name));
                    };
                    let (gp, fty) = self.field_ptr_ty(&sl, &x.fields)?;
                    let dp = self.op(format!("load ptr, ptr {}", gp))?;
                    let lp = self.op(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 1", ARR, gp))?;
                    let ln = self.op(format!("load i64, ptr {}", lp))?;
                    let mut av = V::with_b(Kind::Arr, dp, ln);
                    av.ty = elem_ty(&fty);
                    av
                } else {
                    match bind {
                        Bind::Ssa(sv) => sv,
                        Bind::Slot(sl) => self.load(&sl)?,
                    }
                };
                let g = self.index_ptr(&a, &i)?;
                if a.layout != "i64" {
                    let ty = elem_ir(&a);
                    let z = self.op(format!("trunc i64 {} to {}", v.a, ty))?;
                    self.ins(format!("store {} {}, ptr {}", ty, z, g))?;
                } else if a.ty.as_deref() == Some(ARRAY_BOOL) {
                    // G2 A5: no value range check
                    let z = self.op(format!("zext i1 {} to i8", v.a))?;
                    self.ins(format!("store i8 {}, ptr {}", z, g))?;
                } else if is_u8_ty(&a.ty) {
                    if !self.verified {
                        let ok = self.op(format!("icmp ule i64 {}, 255", v.a))?;
                        self.guard(&ok, Fail::Elem(v.a.clone(), "u8 0..=255"), false)?;
                    }
                    let b = self.op(format!("trunc i64 {} to i8", v.a))?;
                    self.ins(format!("store i8 {}, ptr {}", b, g))?;
                } else {
                    self.ins(format!("store i64 {}, ptr {}", v.a, g))?;
                }
                self.end_temps()
            }
            Stmt::If(x) => {
                self.begin_temps();
                let c = self.ev(&x.cond, false)?;
                self.end_temps()?;
                let thn = self.label("then");
                let end = self.label("endif");
                let has_else = !x.els.is_empty();
                let els = if has_else { self.label("else") } else { end.clone() };
                self.condbr(&c.a, &thn, &els)?;
                self.place(&thn);
                let mut reach = false;
                let prev = std::mem::replace(&mut self.utf8_ok, crate::codegen::utf8_checked(x));
                if self.run_block(&x.then, Vec::new())? {
                    self.br(&end)?;
                    reach = true;
                }
                self.utf8_ok = prev;
                if has_else {
                    self.place(&els);
                    if self.run_block(&x.els, Vec::new())? {
                        self.br(&end)?;
                        reach = true;
                    }
                } else {
                    reach = true;
                }
                if reach {
                    self.place(&end);
                } else {
                    self.dead = true;
                }
                Ok(())
            }
            Stmt::While(x) => {
                let cl = self.label("wcond");
                let bl = self.label("wbody");
                let el = self.label("wend");
                self.br(&cl)?;
                self.place(&cl);
                self.begin_temps();
                let c = self.ev(&x.cond, false)?;
                self.end_temps()?;
                self.condbr(&c.a, &bl, &el)?;
                self.place(&bl);
                if self.cost { self.cost_add("1")?; }
                if self.run_block_in(&x.body, Vec::new(), true)? {
                    self.br(&cl)?;
                }
                self.place(&el);
                Ok(())
            }
            Stmt::ForMap(_) => Err("map iteration needs the Rust backend".into()),
            Stmt::Return(x) => {
                self.begin_temps();
                let mut v = V::unit();
                let tm = match &x.value {
                    Some(val) => self.truncate_move(val)?,
                    None => None,
                };
                if let (true, Some(val)) = (view_ret(self.f), x.value.as_ref()) {
                    // R-6g9-1: a view result borrows its `from` sources; nothing moves out
                    v = self.ev(val, false)?;
                } else if let (Some(sl), Some(Expr::Call(c))) = (tm, x.value.as_ref()) {
                    // P2 batch 4: `return truncate(x, m)` of an owned local array: the same pointer with length m moves out
                    let src = self.load(&sl)?;
                    // LG37: a last-use take inside the length
                    self.moving = self.nested_moves(arg(&c.args, 1)?, None, true);
                    let m = self.ev(arg(&c.args, 1)?, false);
                    self.moving.clear();
                    let m = m?;
                    if !self.verified {
                        self.truncate_guard(&src.b, &m.a)?;
                    }
                    // P3: no runtime call: the same pointer with length m moves out and keeps the source cap, so the
                    // eventual free still gets the real allocation size
                    let mut rv = V::with_b(Kind::Arr, src.a.clone(), m.a.clone());
                    rv.ty = src.ty.clone();
                    rv.layout = src.layout;
                    rv.rng = src.rng;
                    rv.c = Some(capof(&src));
                    v = rv;
                    self.ins(format!("store ptr null, ptr {}", sl.a))?;
                    self.ins(format!("store i64 0, ptr {}", sl.b))?;
                    self.ins(format!("store i64 0, ptr {}", need_c(&sl.c)?))?;
                } else if let Some(val0) = &x.value {
                    // LG9 (G-DB15-2): `return x` of a str match binder (the checker admits it only for a consuming
                    // match) is `return copy(x)`, which moves the payload out of the owned scrutinee (ret_movable)
                    let copied;
                    let val = match val0 {
                        Expr::Var(var) if var.ty.as_deref() == Some(STR)
                            && matches!(self.lookup(&var.name), Ok(Bind::Ssa(_))) => {
                            copied = Expr::Call(Call {
                                line: var.line, col: var.col, name: "copy".to_string(), args: vec![val0.clone()],
                                ty: Some(STR.to_string()), targs: None, inst: None, old: None, opaque_c: false,
                            });
                            &copied
                        }
                        _ => val0,
                    };
                    self.ret_cand = self.ret_movable(val);
                    self.ret_take = self.ret_takable(val);
                    let nm = self.nested_moves(val, None, true);
                    self.moving = nm.difference(&self.ret_cand).cloned().collect();
                    let r = self.ev(val, true);
                    self.moving.clear();
                    v = r?;
                    self.ret_cand.clear();
                    self.ret_take.clear();
                    if v.k == Kind::Arr {
                        if let Expr::Var(var) = val {
                            // When returning a local array out: empty the slot so the cleanup below does not free it
                            let sl = self.lookup(&var.name)?.val().clone();
                            self.ins(format!("store ptr null, ptr {}", sl.a))?;
                            self.ins(format!("store i64 0, ptr {}", sl.b))?;
                            self.ins(format!("store i64 0, ptr {}", need_c(&sl.c)?))?;
                        }
                    }
                }
                self.end_temps()?;
                for i in (0..self.scopes.len()).rev() {
                    let arrays = self.scopes[i].arrays.clone();
                    self.free_scope(&arrays)?;
                }
                self.ret_moved.clear();
                self.emit_ret(&v)
            }
            Stmt::Assert(_) => Ok(()),
            Stmt::Par(p) => self.parallel(p),
            Stmt::ExprStmt(x) => {
                self.begin_temps();
                self.moving = self.nested_moves(&x.expr, None, false);
                let r = self.ev(&x.expr, false);
                self.moving.clear();
                r?;
                self.end_temps()
            }
            Stmt::Match(x) if matches!(&x.scrutinee, Expr::Call(c) if c.name == "vec_at_mut" && native_vec_call_elem(c).is_some()) => {
                self.native_vec_match(x)
            }
            Stmt::Match(x) if expr_ty(&x.scrutinee).as_deref().is_some_and(is_owned_result) => self.owned_match(x),
            Stmt::Match(x) if self.is_ext_opt(expr_ty(&x.scrutinee)) => {
                // G9 A3: the scrutinee is borrowed, never consumed; every temporary it created (a fresh scrutinee, a
                // fresh base of a field path, a fresh argument) is moved into owner slots of a scope that spans all
                // arms, so it lives until the selected arm exits (also through an early return) and is dropped once.
                self.begin_temps();
                let sv = self.ev(&x.scrutinee, false)?;
                let held = self.temps.pop().ok_or_else(|| "pop from empty list".to_string())?;
                self.push_scope();
                for tv in &held {
                    let sl = self.hold(tv)?;
                    self.scopes.last_mut().ok_or_else(|| IDX_ERR.to_string())?.arrays.push(sl);
                }
                self.user_match(x, &sv)?;
                self.pop_scope()
            }
            Stmt::Match(x) => {
                self.begin_temps();
                let sv = self.ev(&x.scrutinee, false)?;
                self.end_temps()?;
                if sv.k == Kind::Agg && sv.ty.as_deref().map_or(false, is_gopt) {
                    // G3: Option<X>/Result<X>, X != i64
                    return self.gmatch(x, &sv);
                }
                if let Some(first) = x.arms.first() {
                    if first.enum_name.is_some() {
                        return self.user_match(x, &sv);
                    }
                }
                let pos = x
                    .arms
                    .iter()
                    .find(|a| a.ctor == "Some" || a.ctor == "Ok")
                    .ok_or_else(|| IDX_ERR.to_string())?;
                let neg = x
                    .arms
                    .iter()
                    .find(|a| a.ctor == "None" || a.ctor == "Err")
                    .ok_or_else(|| IDX_ERR.to_string())?;
                let pl = self.label("armT");
                let nl = self.label("armF");
                let end = self.label("endmatch");
                self.condbr(&sv.a, &pl, &nl)?;
                let mut reach = false;
                for (arm, lbl) in [(pos, &pl), (neg, &nl)] {
                    self.place(lbl);
                    let mut bind: Vec<(String, Bind)> = Vec::new();
                    if let Some(b0) = arm.binders.first() {
                        bind.push((b0.clone(), Bind::Ssa(V::new(Kind::I64, sv.b.clone()))));
                    }
                    if self.run_block(&arm.body, bind)? {
                        self.br(&end)?;
                        reach = true;
                    }
                }
                if reach {
                    self.place(&end);
                } else {
                    self.dead = true;
                }
                Ok(())
            }
        }
    }

    /// match on a user enum: tag switch; payload extraction only inside the arm block; default is unreachable
    pub fn user_match(&mut self, st: &Match, sv: &V) -> R<()> {
        let tt = self.tt;
        let ty = need_ty(&sv.ty)?.to_string();
        let tag = self.tag_of(sv, false)?;
        let mut arms: Vec<(&MatchArm, String)> = Vec::new();
        for a in &st.arms {
            let l = self.label(&format!("arm{}", a.ctor));
            arms.push((a, l));
        }
        let dflt = self.label("matchdef");
        let end = self.label("endmatch");
        // P3: payload binders of a match on an owned extended local that no arm names again may move out on a return
        let mut own: Option<Slot> = None;
        // G-DB16-1: a binder move also needs the owner under cost metering
        if let (Expr::Var(scv), true) = (&st.scrutinee, !self.cost || crate::codegen::match_moves_binder(st)) {
            let bd = self.lookup(&scv.name)?;
            let mut named = HashSet::new();
            for a in &st.arms {
                named.extend(a.binders.iter().cloned());
                crate::refine::stmt_names(&a.body, &mut named);
            }
            if let Bind::Slot(x) = bd {
                if x.k == Kind::Agg && self.owned_slot(&x) && !named.contains(&scv.name) {
                    own = Some(x);
                }
            }
        }
        // an array binder moved by a let / assignment empties its field of the owner, so the owner must be dead too
        // LG35: an enclosing arm's binders still view the payload of a pinned owner, so its fields are never emptied
        let own_dead = match (&own, &st.scrutinee) {
            (Some(_), Expr::Var(scv)) => !self.pinned.contains(&scv.name) && self.dead_after(&scv.name),
            _ => false,
        };
        let saved_pinned = self.pinned.clone();
        // the local a field-path scrutinee (h.e) borrows from is pinned too
        let mut root = &st.scrutinee;
        while let Expr::FieldAccess(fa) = root {
            root = &fa.base;
        }
        if let Expr::Var(v) = root {
            self.pinned.insert(v.name.clone());
        }
        let mut cases: Vec<String> = Vec::new();
        for (a, l) in &arms {
            cases.push(format!("i64 {}, label %{}", t_variant_index(tt, &ty, &a.ctor)?, l));
        }
        self.term(format!("switch i64 {}, label %{} [ {} ]", tag, dflt, cases.join(" ")))?;
        self.place(&dflt);
        self.term("unreachable".to_string())?;
        let mut reach = false;
        for (a, l) in &arms {
            self.place(l);
            let mut bind: Vec<(String, Bind)> = Vec::new();
            let fds = t_fields(tt, &ty, Some(&a.ctor))?;
            for (i, (name, fd)) in a.binders.iter().zip(fds.iter()).enumerate() {
                let mut v = self.getf(sv, &ty, i, fd, Some(&a.ctor), false)?;
                if let Some(o) = &own {
                    if (self.is_ext(&fd.ty) || is_arr_name(&fd.ty) || is_collection(&fd.ty)) && !self.fb(&ty, Some(&a.ctor), &fd.name) {
                        v.mv = Some(Box::new((o.clone(), a.ctor.clone(), i, own_dead)));
                    }
                }
                bind.push((name.clone(), Bind::Ssa(v)));
            }
            if self.run_block(&a.body, bind)? {
                self.br(&end)?;
                reach = true;
            }
        }
        self.pinned = saved_pinned;
        if reach {
            self.place(&end);
        } else {
            self.dead = true;
        }
        Ok(())
    }

    // ---------- code expressions ----------
    /// P3 batch 2: ", !range !N" for a load of an element whose declared range is known ("" otherwise; verified builds only)
    fn range_md(&mut self, layout: &str, rng: Option<(i128, i128)>) -> String {
        if !self.verified {
            return String::new();
        }
        let (lo, hi) = match rng {
            Some(r) => r,
            None => return String::new(),
        };
        let w: u32 = match layout { "u8" | "i8" => 8, "u16" | "i16" => 16, "u32" | "i32" => 32, _ => 64 };
        let unsigned = layout.starts_with('u');
        let (min, max): (i128, i128) = if unsigned { (0, (1i128 << w) - 1) } else { (-(1i128 << (w - 1)), (1i128 << (w - 1)) - 1) };
        if lo < min || hi > max || lo > hi || hi - lo + 1 >= (1i128 << w) {
            return String::new();
        }
        let m = 1i128 << w;
        let red = |x: i128| -> i128 {
            let r = x.rem_euclid(m);
            if r >= m / 2 { r - m } else { r }
        };
        let body = format!("i{w} {}, i{w} {}", red(lo), red(hi + 1), w = w);
        let n = match self.m.range_nodes.iter().position(|s| *s == body) {
            Some(n) => n,
            None => {
                self.m.range_nodes.push(body);
                self.m.range_nodes.len() - 1
            }
        };
        format!(", !range !{}", n)
    }

    fn narrow_load(&mut self, a: &V, ptr: &str, target: &str) -> R<String> {
        let ty = elem_ir(a);
        let md = self.range_md(a.layout, a.rng);
        let x = self.op(format!("load {}, ptr {}{}", ty, ptr, md))?;
        self.op(format!("{} {} {} to {}", if a.layout.starts_with('i') { "sext" } else { "zext" }, ty, x, target))
    }

    pub fn index_ptr(&mut self, a: &V, i: &V) -> R<String> {
        if !self.verified {
            let ok = self.op(format!("icmp ult i64 {}, {}", i.a, a.b))?;
            self.guard(&ok, Fail::Oob(i.a.clone(), a.b.clone()), false)?;
        }
        self.op(format!("getelementptr inbounds {}, ptr {}, i64 {}", elem_ir(a), a.a, i.a))
    }

    /// P2 batch 4 (Python truncate_move): the slot of x when e is `truncate(x, m)` with x an owned local array (not a
    /// parameter or binder) and m does not name x
    fn truncate_move(&self, e: &Expr) -> R<Option<Slot>> {
        let c = match e {
            Expr::Call(c) if !self.cost && c.name == "truncate" && c.args.len() == 2 => c,
            _ => return Ok(None),
        };
        let v = match &c.args[0] {
            Expr::Var(v) => v,
            _ => return Ok(None),
        };
        let mut names: HashSet<String> = HashSet::new();
        crate::refine::expr_names(&c.args[1], &mut names);
        if names.contains(&v.name) {
            return Ok(None);
        }
        match self.lookup(&v.name)? {
            // LG5: a resizable inout parameter's slot is the caller's, so it is copied, not emptied
            Bind::Slot(sl) if sl.k == Kind::Arr && self.owned_slot(&sl) => Ok(Some(sl)),
            _ => Ok(None),
        }
    }

    /// P2 batch 4: pointer to a[i]; --no-verify first checks 0 <= i && i + nb <= len(a) (stop as for a[i])
    fn word_ptr(&mut self, a: &V, i: &V, nb: i64) -> R<String> {
        if !self.verified {
            let t = self.op(format!("sub i64 {}, {}", a.b, nb))?;
            let c1 = self.op(format!("icmp sge i64 {}, 0", i.a))?;
            let c2 = self.op(format!("icmp sle i64 {}, {}", i.a, t))?;
            let ok = self.op(format!("and i1 {}, {}", c1, c2))?;
            self.guard(&ok, Fail::Oob(i.a.clone(), a.b.clone()), false)?;
        }
        self.op(format!("getelementptr inbounds i8, ptr {}, i64 {}", a.a, i.a))
    }

    /// P17a: one memcmp of n * sz bytes; returns the i1 result
    fn eqr_mem(&mut self, pa: &str, pb: &str, cnt: &str, sz: i64) -> R<String> {
        self.m.memcmp = true;
        let nb = if sz == 1 { cnt.to_string() } else { self.op(format!("mul nuw nsw i64 {}, {}", cnt, sz))? };
        let rc = self.op(format!("call i32 @memcmp(ptr {}, ptr {}, i64 {})", pa, pb, nb))?;
        self.op(format!("icmp eq i32 {}, 0", rc))
    }

    /// P17a: the eq_range element loop with early exit; 0 <= n is known; returns the i1 result in the end block
    fn eqr_loop(&mut self, ty: &str, pa: &str, pb: &str, cnt: &str) -> R<String> {
        let pre = self.cur.clone();
        let head = self.label("eqr.head");
        let body = self.label("eqr.body");
        let end = self.label("eqr.end");
        let k = self.tmp();
        let k1 = self.tmp();
        self.br(&head)?;
        self.place(&head);
        self.ins(format!("{} = phi i64 [ 0, %{} ], [ {}, %{} ]", k, pre, k1, body))?;
        let d = self.op(format!("icmp eq i64 {}, {}", k, cnt))?;
        self.condbr(&d, &end, &body)?;
        self.place(&body);
        let ga = self.op(format!("getelementptr inbounds {}, ptr {}, i64 {}", ty, pa, k))?;
        let gb = self.op(format!("getelementptr inbounds {}, ptr {}, i64 {}", ty, pb, k))?;
        let x = self.op(format!("load {}, ptr {}", ty, ga))?;
        let y = self.op(format!("load {}, ptr {}", ty, gb))?;
        let ne = self.op(format!("icmp ne {} {}, {}", ty, x, y))?;
        self.ins(format!("{} = add nuw nsw i64 {}, 1", k1, k))?;
        self.condbr(&ne, &end, &head)?;
        self.place(&end);
        self.op(format!("phi i1 [ true, %{} ], [ false, %{} ]", head, body))
    }

    fn bswap(&mut self, bits: u32, x: &str) -> R<String> {
        self.m.bswap.insert(bits);
        self.op(format!("call i{b} @llvm.bswap.i{b}(i{b} {x})", b = bits, x = x))
    }

    /// pointer to byte k of the word at g
    fn byte_at(&mut self, g: &str, k: i64) -> R<String> {
        if k == 0 {
            Ok(g.to_string())
        } else {
            self.op(format!("getelementptr inbounds i8, ptr {}, i64 {}", g, k))
        }
    }

    /// Read nb bytes at g as an i64 in the requested byte order.
    fn load_word(&mut self, g: &str, nb: i64, little: bool) -> R<String> {
        if nb == 2 || nb == 4 || nb == 8 {
            let bits = (8 * nb) as u32;
            let x = self.op(format!("load i{}, ptr {}, align 1", bits, g))?;
            let x = if little { x } else { self.bswap(bits, &x)? };
            return if nb == 8 { Ok(x) } else { self.op(format!("zext i{} {} to i64", bits, x)) };
        }
        let mut acc: Option<String> = None;
        for k in 0..nb {
            let p = self.byte_at(g, if little { nb - 1 - k } else { k })?;
            let b = self.op(format!("load i8, ptr {}", p))?;
            let mut b = self.op(format!("zext i8 {} to i64", b))?;
            if let Some(a) = acc {
                let sh = self.op(format!("shl i64 {}, 8", a))?;
                b = self.op(format!("or i64 {}, {}", sh, b))?;
            }
            acc = Some(b);
        }
        acc.ok_or_else(|| IDX_ERR.to_string())
    }

    /// Store the low 8*nb bits of w at g in 8/4/2/1-byte pieces.
    fn store_word(&mut self, g: &str, w: &str, nb: i64, little: bool) -> R<()> {
        let mut off: i64 = 0;
        while off < nb {
            let left = nb - off;
            let mut p: i64 = if left >= 4 { 4 } else if left >= 2 { 2 } else { 1 };
            if nb == 1 || nb == 2 || nb == 4 || nb == 8 {
                p = nb;
            }
            let sh = 8 * if little { off } else { left - p };
            let x = if sh == 0 { w.to_string() } else { self.op(format!("lshr i64 {}, {}", w, sh))? };
            if p == 8 {
                let x = if little { x } else { self.bswap(64, &x)? };
                let ptr = self.byte_at(g, off)?;
                self.ins(format!("store i64 {}, ptr {}, align 1", x, ptr))?;
            } else if p == 1 {
                let x = self.op(format!("trunc i64 {} to i8", x))?;
                let ptr = self.byte_at(g, off)?;
                self.ins(format!("store i8 {}, ptr {}", x, ptr))?;
            } else {
                let bits = (8 * p) as u32;
                let x = self.op(format!("trunc i64 {} to i{}", x, bits))?;
                let x = if little { x } else { self.bswap(bits, &x)? };
                let ptr = self.byte_at(g, off)?;
                self.ins(format!("store i{} {}, ptr {}, align 1", bits, x, ptr))?;
            }
            off += p;
        }
        Ok(())
    }

    pub fn checked(&mut self, ty: &str, name: &str, sym: &str, a: &str, b: &str) -> R<String> {
        let r = self.op(format!("call {{ {}, i1 }} @llvm.{}.with.overflow.{}({} {}, {} {})", ty, name, ty, ty, a, ty, b))?;
        let v = self.op(format!("extractvalue {{ {}, i1 }} {}, 0", ty, r))?;
        let o = self.op(format!("extractvalue {{ {}, i1 }} {}, 1", ty, r))?;
        if ty == "i64" {
            self.guard(&o, Fail::Panic1("pruv_panic_arith", format!("overflow in {}", sym)), true)?;
        } else {
            self.guard(&o, Fail::Panic1("pruv_panic_undefined", format!("i128 overflow in {}", sym)), true)?;
        }
        Ok(v)
    }

    /// G2 B2: registers the literal bytes (valid UTF-8) as a global string and returns `@.str.N`.
    fn str_global(&mut self, value: &[u8]) -> String {
        let text = String::from_utf8_lossy(value).into_owned();
        self.m.string(&text)
    }

    pub fn temp_arr(&mut self, v: V, owned: bool) -> R<V> {
        if !owned {
            self.temps.last_mut().ok_or_else(|| IDX_ERR.to_string())?.push(v.clone());
        }
        Ok(v)
    }

    pub fn shortcircuit(&mut self, l: &str, is_or: bool, rhs: &dyn Fn(&mut FnGen<'a>) -> R<String>) -> R<String> {
        let lcur = self.cur.clone();
        let rl = self.label("rhs");
        let el = self.label("endsc");
        if is_or {
            self.condbr(l, &el, &rl)?;
        } else {
            self.condbr(l, &rl, &el)?;
        }
        self.place(&rl);
        let r = rhs(self)?;
        let rcur = self.cur.clone();
        self.br(&el)?;
        self.place(&el);
        self.op(format!("phi i1 [ {}, %{} ], [ {}, %{} ]", if is_or { "true" } else { "false" }, lcur, r, rcur))
    }

    pub fn ev(&mut self, e: &Expr, owned: bool) -> R<V> {
        match e {
            Expr::Spent(_) => Err("cost_unavailable: Stage 5 cost verification is not implemented yet".into()),
            Expr::IntLit(x) => Ok(V::new(Kind::I64, x.value.clone())),
            Expr::BoolLit(x) => Ok(V::new(Kind::I1, if x.value { "true" } else { "false" })),
            Expr::StrLit(x) => {
                // G2 B2: copy the global string into a new buffer (for an empty string pruv_copy_v2 gives NULL)
                let g = self.str_global(&x.value);
                let l = x.value.len().to_string();
                if !owned {
                    // P3: a borrowed literal reads the read-only global directly (nothing to free)
                    let mut rv = V::with_b(Kind::Arr, g, l);
                    rv.ty = x.ty.clone();
                    return Ok(rv);
                }
                let p = self.op(format!("call ptr @pruv_copy_v2(ptr {}, i64 {}, i64 1, i64 1)", g, l))?;
                let mut rv = V::with_b(Kind::Arr, p, l);
                rv.ty = Some(STR.to_string());
                self.temp_arr(rv, owned)
            }
            Expr::Var(x) => {
                let bd = self.lookup(&x.name)?;
                let mut v = match &bd {
                    Bind::Ssa(v) => v.clone(),
                    Bind::Slot(s) => self.load(s)?,
                };
                if owned && v.k == Kind::Agg && self.is_ext_opt(&v.ty) {
                    if self.ret_cand.contains(&x.name) {
                        // P3: moved out by the return (ret_movable)
                        match &bd {
                            Bind::Slot(s) => {
                                self.ret_moved.insert(s.a.clone(), None);
                            }
                            Bind::Ssa(b) => {
                                let mv = b.mv.as_ref().ok_or_else(|| IDX_ERR.to_string())?;
                                let (sl, vn, k) = (&mv.0, &mv.1, mv.2);
                                let mut set = match self.ret_moved.get(&sl.a) {
                                    Some(Some((_, s))) => s.clone(),
                                    _ => HashSet::new(),
                                };
                                set.insert(k);
                                self.ret_moved.insert(sl.a.clone(), Some((vn.clone(), set)));
                            }
                        }
                        return Ok(v);
                    }
                    let ty = need_ty(&v.ty)?;
                    if is_owned_result(ty) {
                        let Bind::Slot(slot) = &bd else {
                            return Err(e2_diag(x.line, x.col, &format!(
                                "moving the borrowed Result<_, String> {} needs the Rust backend", x.name)));
                        };
                        self.ins(format!("store {} zeroinitializer, ptr {}", lty(ty), slot.a))?;
                        v.p = None;
                        return Ok(v);
                    }
                    // G9 A3: a named extended value in an owning position
                    return self.clone_val(&v);
                }
                Ok(v)
            }
            Expr::Move(x) if !x.fields.is_empty() => {
                // LG4: move an owned buffer field out of the extended struct slot; the field becomes (null, 0, 0)
                let sl = self.field_move_root(x)?;
                let (gp, fty) = self.field_ptr_ty(&sl, &x.fields)?;
                let lt = lty(&fty);
                let old = self.op(format!("load {lt}, ptr {gp}"))?;
                let empty = if is_collection(&fty) { "null" } else { "zeroinitializer" };
                self.ins(format!("store {lt} {empty}, ptr {gp}"))?;
                self.unpack(&old, &fty, false)
            }
            Expr::Move(x) => {
                // G6 A3: load the owner slot, then null it
                let bound = self.lookup(&x.name)?;
                if let Bind::Ssa(v) = &bound {
                    if let Some(mv) = &v.mv {
                        if v.ty.as_deref().is_some_and(|t| native_vec_elem(t).is_some() || is_collection(t)) {
                            let g = g9_field_gep(self, need_ty(&mv.0.ty)?, &mv.0.a, Some(&mv.1), mv.2)?;
                            let ty = need_ty(&v.ty)?;
                            let empty = if is_collection(ty) { "null" } else { "zeroinitializer" };
                            self.ins(format!("store {} {empty}, ptr {g}", lty(ty)))?;
                            return Ok(V { p: None, ..v.clone() });
                        }
                    }
                    if v.k == Kind::Arr {
                        if let Some(mv) = &v.mv {
                            // G-DB16-1: moving an array binder of a match on an owned local takes the descriptor and
                            // empties the field of the scrutinee (null, 0, 0), so the scrutinee's drop frees nothing twice
                            let (sl, vn, k, _dead) = (**mv).clone();
                            let g = g9_field_gep(self, need_ty(&sl.ty)?, &sl.a, Some(&vn), k)?;
                            self.ins(format!("store {} zeroinitializer, ptr {}", ARR, g))?;
                            return Ok(v.clone());
                        }
                    }
                }
                let sl = bound.val().clone();
                let mut v = self.load(&sl)?;
                if x.ty.as_deref().is_some_and(|t| is_array_name(t) || t == "str") {
                    self.empty_array_slot(&sl)?;
                } else if sl.k == Kind::Agg && self.is_ext_opt(&sl.ty) {
                    // LG9 (G-LG8-5): an extended record owns every buffer field; the whole slot becomes zero (its drop
                    // then frees nothing) and the moved value is no longer backed by the emptied slot
                    let lt = lty(need_ty(&sl.ty)?);
                    self.ins(format!("store {} zeroinitializer, ptr {}", lt, sl.a))?;
                    v.p = None;
                } else {
                    self.ins(format!("store ptr null, ptr {}", sl.a))?;
                }
                Ok(v)
            }
            // G5 A0: never reaches the generator (the driver's A0 guard stops the program earlier)
            Expr::InoutArg(_) => Err("G5 inout argument reached the LLVM generator".to_string()),
            Expr::Unary(x) => {
                let v = self.ev(&x.operand, false)?;
                if x.op == "!" {
                    let r = self.op(format!("xor i1 {}, true", v.a))?;
                    return Ok(V::new(Kind::I1, r));
                }
                if x.op == "~" {
                    let r = self.op(format!("xor i64 {}, -1", v.a))?;
                    return Ok(V::new(Kind::I64, r));
                }
                if self.verified {
                    let r = self.op(format!("sub nsw i64 0, {}", v.a))?;
                    return Ok(V::new(Kind::I64, r));
                }
                let r = self.checked("i64", "ssub", "unary -", "0", &v.a)?;
                Ok(V::new(Kind::I64, r))
            }
            Expr::Binary(x) => self.ev_binary(x),
            Expr::Index(x) => {
                let a = self.ev(&x.array, false)?;
                let i = self.ev(&x.index, false)?;
                let g = self.index_ptr(&a, &i)?;
                if a.ty.as_deref() == Some(ARRAY_BOOL) {
                    // G2 A5
                    let x8 = self.op(format!("load i8, ptr {}", g))?;
                    let r = self.op(format!("icmp ne i8 {}, 0", x8))?;
                    return Ok(V::new(Kind::I1, r));
                }
                if is_u8_ty(&a.ty) || is_strv(&a.ty) {
                    let x8 = self.op(format!("load i8, ptr {}", g))?;
                    let r = self.op(format!("zext i8 {} to i64", x8))?;
                    return Ok(V::new(Kind::I64, r));
                }
                if a.layout != "i64" {
                    return Ok(V::new(Kind::I64, self.narrow_load(&a, &g, "i64")?));
                }
                let md = self.range_md(a.layout, a.rng);
                let r = self.op(format!("load i64, ptr {}{}", g, md))?;
                Ok(V::new(Kind::I64, r))
            }
            Expr::Ctor(x) => {
                if x.ty.as_deref().is_some_and(is_owned_result) {
                    let value = self.mk_octor(x)?;
                    return self.fresh(value, owned);
                }
                if x.ty.as_deref().map_or(false, is_gopt) {
                    return self.mk_gctor(x, false); // G3
                }
                if x.name == "None" {
                    return Ok(V::with_b(Kind::Opt, "false", "0"));
                }
                let a = x.arg.as_ref().ok_or_else(|| "llvmgen: constructor without argument".to_string())?;
                let v = self.ev(a, false)?;
                Ok(V::with_b(Kind::Opt, if x.name == "Some" || x.name == "Ok" { "true" } else { "false" }, v.a))
            }
            Expr::Call(x) => self.ev_call(x, owned, self.m.storage.layout_of(e), self.m.storage.range_of(e)),
            Expr::StructLit(x) => {
                if self.is_ext(&x.name) {
                    // G9 A3: constructor fields are owning positions; the result is fresh
                    let r = self.mk_struct(x, false, true)?;
                    return self.fresh(r, owned);
                }
                self.mk_struct(x, false, false)
            }
            Expr::EnumCtor(x) => {
                if self.is_ext(&x.enum_name) {
                    let r = self.mk_enum(x, false, true)?;
                    return self.fresh(r, owned);
                }
                self.mk_enum(x, false, false)
            }
            Expr::FieldAccess(x) => {
                if matches!(x.ty.as_deref(), Some("i64" | "bool" | "u8")) {
                    let mut path = vec![x.field.clone()];
                    let mut base = x.base.as_ref();
                    while let Expr::FieldAccess(field) = base {
                        path.push(field.field.clone());
                        base = field.base.as_ref();
                    }
                    if let Expr::Var(var) = base {
                        if let Ok(Bind::Slot(sl)) = self.lookup(&var.name) {
                            if !self.is_ext_opt(&sl.ty)
                                && matches!(sl.ty.as_deref().and_then(|ty| self.tt.decl(ty)), Some(TypeDecl::Struct(_)))
                            {
                                path.reverse();
                                let (gp, fty) = self.field_ptr_ty(&sl, &path)?;
                                let v = self.op(format!("load {}, ptr {}", st(&fty, false), gp))?;
                                return self.unpack(&v, &fty, false);
                            }
                        }
                    }
                }
                let tt = self.tt;
                let b = self.ev(&x.base, false)?;
                let ty = match &b.ty {
                    Some(t) => t.clone(),
                    None => need_ty(expr_ty(&x.base))?.to_string(),
                };
                let i = t_field_index(tt, &ty, &x.field, None)?;
                let fds = t_fields(tt, &ty, None)?;
                let fd = fds.get(i).ok_or_else(|| IDX_ERR.to_string())?;
                let v = self.getf(&b, &ty, i, fd, None, false)?;
                if owned && v.k == Kind::Agg && self.is_ext(&fd.ty) {
                    // G9 A3: a field path in an owning position clones
                    return self.clone_val(&v);
                }
                Ok(v)
            }
            other => Err(format!("llvmgen: contract-only expression in code: {:?}", other)),
        }
    }

    fn guard_shift_count(&mut self, count: &str) -> R<()> {
        let ok = self.op(format!("icmp ult i64 {}, 64", count))?;
        self.guard(&ok, Fail::Panic1("pruv_panic_arith", "shift count outside 0..64".to_string()), false)
    }

    pub fn ev_binary(&mut self, e: &Binary) -> R<V> {
        let op = e.op.as_str();
        if op == "&&" || op == "||" {
            let l = self.ev(&e.left, false)?;
            let right = &e.right;
            let r = self.shortcircuit(&l.a, op == "||", &|g| {
                g.begin_temps();
                let r = g.ev(right, false)?;
                g.end_temps()?;
                Ok(r.a)
            })?;
            return Ok(V::new(Kind::I1, r));
        }
        let l = self.ev(&e.left, false)?;
        let r = self.ev(&e.right, false)?;
        if let Some(ins) = bit_operator_llvm(op) {
            if !self.verified && matches!(op, "<<" | ">>") {
                self.guard_shift_count(&r.a)?;
            }
            let v = self.op(format!("{} i64 {}, {}", ins, l.a, r.a))?;
            if !self.verified && op == "<<" {
                let back = self.op(format!("ashr i64 {}, {}", v, r.a))?;
                let overflow = self.op(format!("icmp ne i64 {}, {}", back, l.a))?;
                self.guard(&overflow, Fail::Panic1("pruv_panic_arith", "overflow in <<".to_string()), true)?;
            }
            return Ok(V::new(Kind::I64, v));
        }
        if op == "+" || op == "-" || op == "*" {
            let name = match op {
                "+" => "add",
                "-" => "sub",
                _ => "mul",
            };
            if self.verified {
                let v = self.op(format!("{} nsw i64 {}, {}", name, l.a, r.a))?;
                return Ok(V::new(Kind::I64, v));
            }
            let v = self.checked("i64", &format!("s{}", name), op, &l.a, &r.a)?;
            return Ok(V::new(Kind::I64, v));
        }
        if op == "/" || op == "%" {
            if !self.verified {
                let c1 = self.op(format!("icmp sgt i64 {}, 0", r.a))?;
                let c2 = self.op(format!("icmp sge i64 {}, 0", l.a))?;
                let ok = self.op(format!("and i1 {}, {}", c1, c2))?;
                self.guard(
                    &ok,
                    Fail::Panic1("pruv_panic_arith", "division outside domain: need divisor > 0 and dividend >= 0".to_string()),
                    false,
                )?;
            }
            let v = self.op(format!("{} i64 {}, {}", if op == "/" { "udiv" } else { "urem" }, l.a, r.a))?;
            return Ok(V::new(Kind::I64, v));
        }
        let cmp = cmp_name(op)?;
        let ty = if l.k == Kind::I1 { "i1" } else { "i64" };
        let v = self.op(format!("icmp {} {} {}, {}", cmp, ty, l.a, r.a))?;
        Ok(V::new(Kind::I1, v))
    }

    pub fn ev_call(&mut self, e: &Call, owned: bool, layout: &'static str, rng: Option<(i128, i128)>) -> R<V> {
        let result = self.ev_call_inner(e, owned, layout, rng)?;
        if self.cost && matches!(e.name.as_str(), "alloc" | "copy" | "truncate" | "resize") {
            let length = self.op(format!("zext i64 {} to i128", result.b))?;
            let units = self.op(format!("add i128 {}, 1", length))?;
            self.cost_add(&units)?;
        }
        Ok(result)
    }

    fn ev_call_inner(&mut self, e: &Call, owned: bool, layout: &'static str, rng: Option<(i128, i128)>) -> R<V> {
        let n = e.name.as_str();
        let a = &e.args;
        if n == "len" {
            let x = self.ev(arg(a, 0)?, false)?;
            if x.ty.as_deref().and_then(native_vec_elem).is_some() {
                let len = self.op(format!("extractvalue {ARR} {}, 1", x.a))?;
                return Ok(V::new(Kind::I64, len));
            }
            if x.k == Kind::Coll {
                // G6 A3
                let ty = need_ty(&x.ty)?.to_string();
                let sym = crate::foreign_shim::len_sym(&ty)?;
                let r = self.g6_call(&sym, "i64", &[("ptr".to_string(), x.a.clone())])?;
                return Ok(V::new(Kind::I64, r));
            }
            return Ok(V::new(Kind::I64, x.b));
        }
        if n == "string_to_str" && !self.funcs.contains_key(n) {
            // P3 M5: string_to_str of an unboxed String local (an owned str array): move it out in a take position
            // (M3 conditions), else copy it; the other paths free the array with the scope
            if let Some(Expr::Var(x)) = a.first() {
                if let Ok(Bind::Slot(sl)) = self.lookup(&x.name) {
                    if sl.k == Kind::Arr {
                        self.m.g6_v2 = true;
                        let v = self.ev(&a[0], false)?;
                        if !self.cost && self.ret_take.contains(&x.name) {
                            self.ret_moved.insert(sl.a.clone(), None); // the return's cleanup skips the slot
                            return self.temp_arr(v, owned);
                        }
                        if !self.cost && self.take_let.contains(&x.name) {
                            self.ins(format!("store ptr null, ptr {}", sl.a))?;
                            self.ins(format!("store i64 0, ptr {}", sl.b))?;
                            self.ins(format!("store i64 0, ptr {}", need_c(&sl.c)?))?;
                            return self.temp_arr(v, owned);
                        }
                        let p = self.op(format!("call ptr @pruv_copy_v2(ptr {}, i64 {}, i64 1, i64 1)", v.a, v.b))?;
                        let mut rv = V::with_b(Kind::Arr, p, v.b.clone());
                        rv.ty = Some(STR.to_string());
                        return self.temp_arr(rv, owned);
                    }
                }
            }
        }
        if crate::foreign::is_catalog(n) && !crate::foreign::is_catalog_contract_only(n) && !self.funcs.contains_key(n) {
            // G6 A3: reserved names
            return self.catalog_call(e, owned);
        }
        if n == "alloc" {
            let cnt = self.ev(arg(a, 0)?, false)?; // source order in all modes: n, then v
            let v = self.ev(arg(a, 1)?, false)?;
            let u8a = is_u8_ty(&e.ty);
            if !self.verified {
                let ok = self.op(format!("icmp ult i64 {}, 1099511627777", cnt.a))?;
                self.guard(&ok, Fail::Panic1("pruv_panic_arith", "alloc length outside 0..2^40".to_string()), false)?;
                if u8a {
                    let ok2 = self.op(format!("icmp ule i64 {}, 255", v.a))?;
                    self.guard(&ok2, Fail::Elem(v.a.clone(), "u8 0..=255"), false)?;
                }
            }
            if e.ty.as_deref() == Some(ARRAY_BOOL) {
                // G2 A5: zext i1 -> i64, allocate with pruv_alloc_u8
                let z = self.op(format!("zext i1 {} to i64", v.a))?;
                let p = self.op(format!("call ptr @pruv_alloc_u8(i64 {}, i64 {})", cnt.a, z))?;
                let mut rv = V::with_b(Kind::Arr, p, cnt.a.clone());
                rv.ty = Some(ARRAY_BOOL.to_string());
                return self.temp_arr(rv, owned);
            }
            let tail = match &self.tail {
                Some((at, k)) if *at == e as *const Call as usize => Some(k.clone()),
                _ => None,
            };
            if u8a {
                if let Some(k) = &tail {
                    // P3: tail_fill
                    let kv = self.ev(k, false)?;
                    self.m.tail_u8 = true;
                    let p = self.op(format!("call ptr @pruv_alloc_tail_u8(i64 {}, i64 {})", cnt.a, kv.a))?;
                    let mut rv = V::with_b(Kind::Arr, p, cnt.a.clone());
                    rv.ty = Some(ARRAY_U8.to_string());
                    return self.temp_arr(rv, owned);
                }
                let p = self.op(format!("call ptr @pruv_alloc_u8(i64 {}, i64 {})", cnt.a, v.a))?;
                let mut rv = V::with_b(Kind::Arr, p, cnt.a.clone());
                rv.ty = Some(ARRAY_U8.to_string());
                return self.temp_arr(rv, owned);
            }
            if layout != "i64" {
                let p = self.op(format!("call ptr @pruv_alloc_narrow(i64 {}, i64 {}, i64 {}, i64 {})", cnt.a, v.a, elem_size(layout), alignment(layout)))?;
                let mut rv = V::with_b(Kind::Arr, p, cnt.a.clone());
                rv.layout = layout;
                rv.rng = rng;
                return self.temp_arr(rv, owned);
            }
            if let Some(k) = &tail {
                // P3: tail_fill
                let kv = self.ev(k, false)?;
                self.m.tail_i64 = true;
                let p = self.op(format!("call ptr @pruv_alloc_tail(i64 {}, i64 {})", cnt.a, kv.a))?;
                let mut rv = V::with_b(Kind::Arr, p, cnt.a.clone());
                rv.rng = rng;
                return self.temp_arr(rv, owned);
            }
            let p = self.op(format!("call ptr @pruv_alloc(i64 {}, i64 {})", cnt.a, v.a))?;
            let mut rv = V::with_b(Kind::Arr, p, cnt.a.clone());
            rv.rng = rng;
            return self.temp_arr(rv, owned);
        }
        if n == "copy" {
            if let Some(mv) = self.copy_move(arg(a, 0)?)? {
                return self.temp_arr(mv, owned);
            }
            let src = self.ev(arg(a, 0)?, false)?;
            if src.layout != "i64" {
                let p = self.op(format!("call ptr @pruv_copy_v2(ptr {}, i64 {}, i64 {}, i64 {})", src.a, src.b, elem_size(src.layout), alignment(src.layout)))?;
                let mut rv = V::with_b(Kind::Arr, p, src.b.clone());
                rv.ty = src.ty.clone();
                rv.layout = src.layout;
                rv.rng = src.rng;
                return self.temp_arr(rv, owned);
            }
            if is_bytes_ty(&src.ty) {
                let p = self.op(format!("call ptr @pruv_copy_v2(ptr {}, i64 {}, i64 1, i64 1)", src.a, src.b))?;
                let mut rv = V::with_b(Kind::Arr, p, src.b.clone());
                rv.ty = src.ty.clone();
                rv.layout = src.layout;
                rv.rng = src.rng;
                return self.temp_arr(rv, owned);
            }
            let p = self.op(format!("call ptr @pruv_copy(ptr {}, i64 {})", src.a, src.b))?;
            let mut rv = V::with_b(Kind::Arr, p, src.b.clone());
            rv.rng = src.rng;
            return self.temp_arr(rv, owned);
        }
        if n == "truncate" {
            // the first m elements of the array as a new array
            let src = self.ev(arg(a, 0)?, false)?;
            let m = self.ev(arg(a, 1)?, false)?;
            if !self.verified {
                self.truncate_guard(&src.b, &m.a)?;
            }
            if let Expr::Var(xv) = arg(a, 0)? {
                if self.ret_cand.contains(&xv.name) {
                    if let Bind::Slot(sl) = self.lookup(&xv.name)? {
                        if sl.k == Kind::Arr {
                            // P3: an owned local array the return moves (ret_movable): the same block with length m and
                            // the source cap moves out (no runtime call), as `return truncate(x, m)` does; the
                            // return's cleanup skips the slot
                            self.ret_moved.insert(sl.a.clone(), None);
                            let mut rv = V::with_b(Kind::Arr, src.a.clone(), m.a.clone());
                            rv.ty = src.ty.clone();
                            rv.layout = src.layout;
                            rv.rng = src.rng;
                            rv.c = Some(capof(&src));
                            return self.temp_arr(rv, owned);
                        }
                    }
                }
            }
            // LG33 (G-LG32-5): an owned local array at its last use (move_src) keeps its block with length m; as the Rust
            // backend's truncate move, a block left more than half unused is shrunk to m (pruv_shrink_v2 returns the
            // block unchanged when the target is the cap, frees it and returns NULL only when the target is 0, and
            // panics if a shrinking realloc fails, so the recorded cap is always the block's real cap)
            if let Expr::Var(xv) = arg(a, 0)? {
                if self.moving.contains(&xv.name) {
                    self.take_array(&xv.name)?;
                    let cap = capof(&src);
                    let half = self.op(format!("lshr i64 {}, 1", cap))?;
                    let cut = self.op(format!("icmp ugt i64 {}, {}", half, m.a))?;
                    let nc = self.op(format!("select i1 {}, i64 {}, i64 {}", cut, m.a, cap))?;
                    self.m.shrink = true;
                    let (sz, al) = if is_bytes_ty(&src.ty) { (1, 1) } else { (elem_size(src.layout), alignment(src.layout)) };
                    let p = self.op(format!("call ptr @pruv_shrink_v2(ptr {}, i64 {}, i64 {}, i64 {}, i64 {})", src.a, cap, nc, sz, al))?;
                    let mut rv = V::with_b(Kind::Arr, p, m.a.clone());
                    rv.ty = src.ty.clone();
                    rv.layout = src.layout;
                    rv.rng = src.rng;
                    rv.c = Some(nc);
                    return self.temp_arr(rv, owned);
                }
            }
            if src.layout != "i64" {
                let p = self.op(format!("call ptr @pruv_copy_v2(ptr {}, i64 {}, i64 {}, i64 {})", src.a, m.a, elem_size(src.layout), alignment(src.layout)))?;
                let mut rv = V::with_b(Kind::Arr, p, m.a.clone());
                rv.ty = src.ty.clone();
                rv.layout = src.layout;
                rv.rng = src.rng;
                return self.temp_arr(rv, owned);
            }
            if is_bytes_ty(&src.ty) {
                let p = self.op(format!("call ptr @pruv_copy_v2(ptr {}, i64 {}, i64 1, i64 1)", src.a, m.a))?;
                let mut rv = V::with_b(Kind::Arr, p, m.a.clone());
                rv.ty = src.ty.clone();
                rv.layout = src.layout;
                rv.rng = src.rng;
                return self.temp_arr(rv, owned);
            }
            let p = self.op(format!("call ptr @pruv_copy(ptr {}, i64 {})", src.a, m.a))?;
            let mut rv = V::with_b(Kind::Arr, p, m.a.clone());
            rv.rng = src.rng;
            return self.temp_arr(rv, owned);
        }
        if n == "resize" {
            // P05G: a grown copy: source order a, n, v; one allocation, prefix memcpy, tail fill
            let src = self.ev(arg(a, 0)?, false)?;
            let cnt = self.ev(arg(a, 1)?, false)?;
            let v = self.ev(arg(a, 2)?, false)?;
            let u8a = src.ty.as_deref() == Some(ARRAY_U8);
            self.resize_guards(&src, &cnt, &v, false)?;
            if let Expr::Var(x) = arg(a, 0)? {
                if self.moving.contains(&x.name) {
                    // LG37 (G-DB49-2): an owned local array at its last use (nested_moves): its block grows in place
                    // (realloc when n > cap, the tail filled with v) and moves into the result, as `x = resize(x, n, v)`
                    self.take_array(&x.name)?;
                    let (q, nc) = self.regrow_block(&src, &cnt, &v)?;
                    let mut rv = V::with_b(Kind::Arr, q, cnt.a.clone());
                    rv.ty = src.ty.clone();
                    rv.layout = src.layout;
                    rv.rng = src.rng;
                    rv.c = Some(nc);
                    return self.temp_arr(rv, owned);
                }
            }
            if src.ty.as_deref() == Some(ARRAY_BOOL) {
                let z = self.op(format!("zext i1 {} to i64", v.a))?;
                self.m.resize_u8 = true;
                let p = self.op(format!("call ptr @pruv_resize_u8(ptr {}, i64 {}, i64 {}, i64 {})", src.a, src.b, cnt.a, z))?;
                let mut rv = V::with_b(Kind::Arr, p, cnt.a.clone());
                rv.ty = Some(ARRAY_BOOL.to_string());
                return self.temp_arr(rv, owned);
            }
            if u8a {
                self.m.resize_u8 = true;
                let p = self.op(format!("call ptr @pruv_resize_u8(ptr {}, i64 {}, i64 {}, i64 {})", src.a, src.b, cnt.a, v.a))?;
                let mut rv = V::with_b(Kind::Arr, p, cnt.a.clone());
                rv.ty = Some(ARRAY_U8.to_string());
                return self.temp_arr(rv, owned);
            }
            if src.layout != "i64" {
                // a narrow layout: the same single pass at the element's width (prefix memcpy, tail fill)
                self.m.resize_narrow = true;
                let p = self.op(format!(
                    "call ptr @pruv_resize_narrow(ptr {}, i64 {}, i64 {}, i64 {}, i64 {}, i64 {})",
                    src.a, src.b, cnt.a, v.a, elem_size(src.layout), alignment(src.layout)
                ))?;
                let mut rv = V::with_b(Kind::Arr, p, cnt.a.clone());
                rv.ty = src.ty.clone();
                rv.layout = src.layout;
                rv.rng = src.rng;
                return self.temp_arr(rv, owned);
            }
            self.m.resize_i64 = true;
            let p = self.op(format!("call ptr @pruv_resize(ptr {}, i64 {}, i64 {}, i64 {})", src.a, src.b, cnt.a, v.a))?;
            let mut rv = V::with_b(Kind::Arr, p, cnt.a.clone());
            rv.rng = src.rng;
            return self.temp_arr(rv, owned);
        }
        if n == "print" {
            let x = self.ev(arg(a, 0)?, false)?;
            if x.k == Kind::I1 {
                let z = self.op(format!("zext i1 {} to i64", x.a))?;
                self.ins(format!("call void @pruv_print_bool(i64 {})", z))?;
            } else {
                self.ins(format!("call void @pruv_print_i64(i64 {})", x.a))?;
            }
            return Ok(V::unit());
        }
        if n == "is_char_boundary" {
            // G2 B2: argument order s -> i
            let s = self.ev(arg(a, 0)?, false)?;
            let i = self.ev(arg(a, 1)?, false)?;
            let r = self.op(format!("call i32 @pruv_str_boundary(ptr {}, i64 {}, i64 {})", s.a, s.b, i.a))?;
            let c = self.op(format!("icmp ne i32 {}, 0", r))?;
            return Ok(V::new(Kind::I1, c));
        }
        if let Some(ins) = bit_llvm(n) {
            // P2: no flags, same IR with and without --no-verify
            let x = self.ev(arg(a, 0)?, false)?;
            let y = self.ev(arg(a, 1)?, false)?;
            if !self.verified && (n == "wrapping_shl" || n == "ushr") {
                // P2 batch 2: check the count before shifting
                self.guard_shift_count(&y.a)?;
            }
            let r = self.op(format!("{} i64 {}, {}", ins, x.a, y.a))?;
            return Ok(V::new(Kind::I64, r));
        }
        if crate::check::array_grow_builtin(n) {
            return self.array_grow_stmt(e);
        }
        if matches!(n, "copy_range" | "copy_within") {
            let target = match arg(a, 0)? {
                Expr::InoutArg(ia) => ia.expr.as_ref(),
                other => other,
            };
            let dv = self.ev(target, false)?;
            let iv = self.ev(arg(a, 1)?, false)?;
            let within = n == "copy_within";
            // LG5: a copy_range whose source is the destination place reuses its loaded view and copies by memmove
            let overlap = within || crate::check::same_place(target, arg(a, 2)?);
            let (sv, jv, cnt) = if within {
                (dv.clone(), self.ev(arg(a, 2)?, false)?, self.ev(arg(a, 3)?, false)?)
            } else if overlap {
                (dv.clone(), self.ev(arg(a, 3)?, false)?, self.ev(arg(a, 4)?, false)?)
            } else {
                (self.ev(arg(a, 2)?, false)?, self.ev(arg(a, 3)?, false)?, self.ev(arg(a, 4)?, false)?)
            };
            if dv.k != Kind::Arr || sv.k != Kind::Arr {
                return Err(format!("{}{n}:{}:{}:operand", CPR_DIAG_TAG, e.line, e.col));
            }
            if !self.verified {
                // P13i2a.audit-1: the destination check (i < 0 || n < 0 || i > len(dst) - n) reports (i, len(dst)),
                // then the source check (j < 0 || j > len(src) - n) reports (j, len(src))
                let ta = self.op(format!("sub i64 {}, {}", dv.b, cnt.a))?;
                let c1 = self.op(format!("icmp slt i64 {}, 0", iv.a))?;
                let c3 = self.op(format!("icmp slt i64 {}, 0", cnt.a))?;
                let c4 = self.op(format!("icmp sgt i64 {}, {}", iv.a, ta))?;
                let o = self.op(format!("or i1 {}, {}", c1, c3))?;
                let o = self.op(format!("or i1 {}, {}", o, c4))?;
                self.guard(&o, Fail::Oob(iv.a.clone(), dv.b.clone()), true)?;
                let tb = self.op(format!("sub i64 {}, {}", sv.b, cnt.a))?;
                let c2 = self.op(format!("icmp slt i64 {}, 0", jv.a))?;
                let c5 = self.op(format!("icmp sgt i64 {}, {}", jv.a, tb))?;
                let o = self.op(format!("or i1 {}, {}", c2, c5))?;
                self.guard(&o, Fail::Oob(jv.a.clone(), sv.b.clone()), true)?;
            }
            let tyd = elem_ir(&dv);
            let tys = elem_ir(&sv);
            if !((is_bytes_ty(&dv.ty) && is_bytes_ty(&sv.ty)) || (tyd == "i64" && tys == "i64")) {
                // the storage plan pins copy_range operands to i64 or bytes, as for eq_range; any other pair is not lowered
                return Err(format!("{}{n}:{}:{}:array layout", CPR_DIAG_TAG, e.line, e.col));
            }
            let pd = self.op(format!("getelementptr inbounds {}, ptr {}, i64 {}", tyd, dv.a, iv.a))?;
            let ps = self.op(format!("getelementptr inbounds {}, ptr {}, i64 {}", tys, sv.a, jv.a))?;
            let nb = if tyd == "i8" { cnt.a.clone() } else { self.op(format!("mul nuw nsw i64 {}, 8", cnt.a))? };
            let intrinsic = if overlap { "memmove" } else { "memcpy" };
            self.ins(format!("call void @llvm.{intrinsic}.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)", pd, ps, nb))?;
            return Ok(V::unit());
        }
        if n == "find_byte" {
            // LB6 MC (R-6g5-3): the first b in a[lo..hi] via memchr, or hi; source order a, b, lo, hi (llvmgen.py)
            let av = self.ev(arg(a, 0)?, false)?;
            let bv = self.ev(arg(a, 1)?, false)?;
            let lo = self.ev(arg(a, 2)?, false)?;
            let hi = self.ev(arg(a, 3)?, false)?;
            if av.k != Kind::Arr || !is_bytes_ty(&av.ty) {
                return Err("find_byte is not supported by the llvm codegen layer for this operand yet".to_string());
            }
            if !self.verified {
                let c1 = self.op(format!("icmp slt i64 {}, 0", lo.a))?;
                let c2 = self.op(format!("icmp sgt i64 {}, {}", lo.a, hi.a))?;
                let c3 = self.op(format!("icmp sgt i64 {}, {}", hi.a, av.b))?;
                let o = self.op(format!("or i1 {}, {}", c1, c2))?;
                let o = self.op(format!("or i1 {}, {}", o, c3))?;
                self.guard(&o, Fail::Oob(hi.a.clone(), av.b.clone()), true)?;
                let o = self.op(format!("icmp ugt i64 {}, 255", bv.a))?;
                self.guard(&o, Fail::Elem(bv.a.clone(), "u8 0..=255"), true)?;
            }
            self.m.memchr = true;
            let n = self.op(format!("sub nsw i64 {}, {}", hi.a, lo.a))?;
            // an empty range has no match; alloc(0) may give a null data pointer, and memchr(null, _, 0) is UB
            let z = self.op(format!("icmp eq i64 {}, 0", n))?;
            let pre = self.cur.clone();
            let lcall = self.label("fb.call");
            let ljoin = self.label("fb.join");
            self.condbr(&z, &ljoin, &lcall)?;
            self.place(&lcall);
            let p = self.op(format!("getelementptr inbounds i8, ptr {}, i64 {}", av.a, lo.a))?;
            let b32 = self.op(format!("trunc i64 {} to i32", bv.a))?;
            let q = self.op(format!("call ptr @memchr(ptr {}, i32 {}, i64 {})", p, b32, n))?;
            let isnull = self.op(format!("icmp eq ptr {}, null", q))?;
            let qi = self.op(format!("ptrtoint ptr {} to i64", q))?;
            let bi = self.op(format!("ptrtoint ptr {} to i64", av.a))?;
            let d = self.op(format!("sub nsw i64 {}, {}", qi, bi))?;
            let r1 = self.op(format!("select i1 {}, i64 {}, i64 {}", isnull, hi.a, d))?;
            let lend = self.cur.clone();
            self.br(&ljoin)?;
            self.place(&ljoin);
            let r = self.op(format!("phi i64 [ {}, %{} ], [ {}, %{} ]", hi.a, pre, r1, lend))?;
            return Ok(V::new(Kind::I64, r));
        }
        if n == "eq_range" {
            // P17a: a[i..i+n] == b[j..j+n]; source order a, i, b, j, n
            let av = self.ev(arg(a, 0)?, false)?;
            let iv = self.ev(arg(a, 1)?, false)?;
            let bv = self.ev(arg(a, 2)?, false)?;
            let jv = self.ev(arg(a, 3)?, false)?;
            let cnt = self.ev(arg(a, 4)?, false)?;
            if av.k != Kind::Arr || bv.k != Kind::Arr {
                return Err(format!("{}{}:{}:operand", EQR_DIAG_TAG, e.line, e.col));
            }
            if !self.verified {
                // one combined check: i < 0 || j < 0 || n < 0 || i > len(a) - n || j > len(b) - n
                let ta = self.op(format!("sub i64 {}, {}", av.b, cnt.a))?;
                let tb = self.op(format!("sub i64 {}, {}", bv.b, cnt.a))?;
                let c1 = self.op(format!("icmp slt i64 {}, 0", iv.a))?;
                let c2 = self.op(format!("icmp slt i64 {}, 0", jv.a))?;
                let c3 = self.op(format!("icmp slt i64 {}, 0", cnt.a))?;
                let c4 = self.op(format!("icmp sgt i64 {}, {}", iv.a, ta))?;
                let c5 = self.op(format!("icmp sgt i64 {}, {}", jv.a, tb))?;
                let o = self.op(format!("or i1 {}, {}", c1, c2))?;
                let o = self.op(format!("or i1 {}, {}", o, c3))?;
                let o = self.op(format!("or i1 {}, {}", o, c4))?;
                let o = self.op(format!("or i1 {}, {}", o, c5))?;
                self.guard(&o, Fail::Oob(iv.a.clone(), av.b.clone()), true)?;
            }
            let tya = elem_ir(&av);
            let tyb = elem_ir(&bv);
            if !((is_bytes_ty(&av.ty) && is_bytes_ty(&bv.ty)) || (tya == "i64" && tyb == "i64")) {
                // the storage plan pins eq_range operands to i64 or bytes; any other pair is not lowered
                return Err(format!("{}{}:{}:array layout", EQR_DIAG_TAG, e.line, e.col));
            }
            if matches!(arg(a, 4)?, Expr::IntLit(_)) && cnt.a == "0" {
                // an empty range is equal; alloc(0) may give a null data pointer, and memcmp(null, null, 0) is UB
                return Ok(V::new(Kind::I1, "true".to_string()));
            }
            let pa = self.op(format!("getelementptr inbounds {}, ptr {}, i64 {}", tya, av.a, iv.a))?;
            let pb = self.op(format!("getelementptr inbounds {}, ptr {}, i64 {}", tyb, bv.a, jv.a))?;
            let sz: i64 = if tya == "i8" { 1 } else { 8 };
            // Measured against hand Rust slice == (work/p16/perf, docs/L3_RESTRUCTURE_SPEC.md Appendix P16/P17a):
            // a literal n is a constant-size memcmp, which LLVM expands into wide loads; a dynamic n below
            // EQR_LOOP_MAX elements takes the inline loop, otherwise memcmp.
            let r;
            if matches!(arg(a, 4)?, Expr::IntLit(_)) {
                r = self.eqr_mem(&pa, &pb, &cnt.a, sz)?;
            } else {
                // the memcmp block is the taken successor: with the loop first, the in-loop memcmp
                // path measured 1.2x hand Rust (block layout around the call)
                let big = self.op(format!("icmp uge i64 {}, {}", cnt.a, EQR_LOOP_MAX))?;
                let lmem = self.label("eqr.mem");
                let lsmall = self.label("eqr.small");
                let ljoin = self.label("eqr.join");
                self.condbr(&big, &lmem, &lsmall)?;
                self.place(&lmem);
                let rm = self.eqr_mem(&pa, &pb, &cnt.a, sz)?;
                self.br(&ljoin)?;
                self.place(&lsmall);
                let rl = self.eqr_loop(tya, &pa, &pb, &cnt.a)?;
                let lend = self.cur.clone();
                self.br(&ljoin)?;
                self.place(&ljoin);
                r = self.op(format!("phi i1 [ {}, %{} ], [ {}, %{} ]", rm, lmem, rl, lend))?;
            }
            return Ok(V::new(Kind::I1, r));
        }
        if matches!(n, "load_be" | "load_le") {
            // P2 batch 4: array, then index
            let arr = self.ev(arg(a, 0)?, false)?;
            let i = self.ev(arg(a, 1)?, false)?;
            let nb = crate::check::word_width(arg(a, 2)?).unwrap_or(0);
            let g = self.word_ptr(&arr, &i, nb)?;
            let r = self.load_word(&g, nb, n == "load_le")?;
            return Ok(V::new(Kind::I64, r));
        }
        if matches!(n, "store_be" | "store_le") {
            // P2 batch 4: index, value, then the array
            let i = self.ev(arg(a, 1)?, false)?;
            let w = self.ev(arg(a, 2)?, false)?;
            let target = match arg(a, 0)? {
                Expr::InoutArg(ia) => ia.expr.as_ref(),
                other => other,
            };
            let arr = self.ev(target, false)?;
            let nb = crate::check::word_width(arg(a, 3)?).unwrap_or(0);
            let g = self.word_ptr(&arr, &i, nb)?;
            self.store_word(&g, &w.a, nb, n == "store_le")?;
            return Ok(V::unit());
        }
        if n == "utf8_valid" {
            let b = self.ev(arg(a, 0)?, false)?;
            self.m.uses_utf8_valid = true;
            let ok = self.op(format!("call i1 @pruv_utf8_valid(ptr {}, i64 {})", b.a, b.b))?;
            return Ok(V::new(Kind::I1, ok));
        }
        if n == "str_from_utf8" {
            let b = self.ev(arg(a, 0)?, false)?;
            if !self.verified && e as *const Call as usize != self.utf8_ok {
                self.m.uses_utf8_valid = true;
                self.m.uses_utf8_panic = true;
                let ok = self.op(format!("call i1 @pruv_utf8_valid(ptr {}, i64 {})", b.a, b.b))?;
                self.guard(&ok, Fail::Utf8(b.b.clone()), false)?;
            }
            // LG33 (G-DB54-2): an owned local [u8] at its last use (move_src): the str takes its block and cap
            if let Expr::Var(xv) = arg(a, 0)? {
                if self.moving.contains(&xv.name) {
                    self.take_array(&xv.name)?;
                    let mut rv = V::with_b(Kind::Arr, b.a.clone(), b.b.clone());
                    rv.ty = Some(STR.to_string());
                    rv.c = Some(capof(&b));
                    return self.temp_arr(rv, owned);
                }
            }
            let p = self.op(format!("call ptr @pruv_copy_v2(ptr {}, i64 {}, i64 1, i64 1)", b.a, b.b))?;
            let mut rv = V::with_b(Kind::Arr, p, b.b);
            rv.ty = Some(STR.to_string());
            return self.temp_arr(rv, owned);
        }
        if n == "subview" {
            let source = arg(a, 0)?;
            let mut base = self.ev(source, false)?;
            let lo = self.ev(arg(a, 1)?, false)?;
            let hi = self.ev(arg(a, 2)?, false)?;
            let string_view = e.ty.as_deref() == Some(STR);
            let string_source = string_view && expr_ty(source).as_deref() == Some(STR);
            if !self.verified {
                let nonneg = self.op(format!("icmp sge i64 {}, 0", lo.a))?;
                let ordered = self.op(format!("icmp sle i64 {}, {}", lo.a, hi.a))?;
                let upper = self.op(format!("icmp sle i64 {}, {}", hi.a, base.b))?;
                let ok = self.op(format!("and i1 {}, {}", nonneg, ordered))?;
                let ok = self.op(format!("and i1 {}, {}", ok, upper))?;
                if string_source {
                    let fail = || Fail::Substr(base.a.clone(), base.b.clone(), lo.a.clone(), hi.a.clone());
                    self.guard(&ok, fail(), false)?;
                    let b1 = self.op(format!("call i32 @pruv_str_boundary(ptr {}, i64 {}, i64 {})", base.a, base.b, lo.a))?;
                    let b2 = self.op(format!("call i32 @pruv_str_boundary(ptr {}, i64 {}, i64 {})", base.a, base.b, hi.a))?;
                    let c1 = self.op(format!("icmp ne i32 {}, 0", b1))?;
                    let c2 = self.op(format!("icmp ne i32 {}, 0", b2))?;
                    let ok = self.op(format!("and i1 {}, {}", c1, c2))?;
                    self.guard(&ok, fail(), false)?;
                } else {
                    self.guard(&ok, Fail::Oob(lo.a.clone(), base.b.clone()), false)?;
                    if string_view {
                        // same order as the py compiler: the UTF-8 check on the span, then the view itself
                        self.m.uses_utf8_valid = true;
                        self.m.uses_utf8_panic = true;
                        let length = self.op(format!("sub i64 {}, {}", hi.a, lo.a))?;
                        let g = self.op(format!("getelementptr inbounds i8, ptr {}, i64 {}", base.a, lo.a))?;
                        let okv = self.op(format!("call i1 @pruv_utf8_valid(ptr {}, i64 {})", g, length))?;
                        self.guard(&okv, Fail::Utf8(length), false)?;
                    }
                }
            }
            let length = self.op(format!("sub i64 {}, {}", hi.a, lo.a))?;
            base.a = self.op(format!("getelementptr inbounds {}, ptr {}, i64 {}", elem_ir(&base), base.a, lo.a))?;
            base.b = length;
            base.p = None;
            base.c = None;
            if string_view {
                base.ty = Some(STR.to_string());
            }
            return Ok(base);
        }
        if n == "substr" {
            // G2 B2: argument order s -> lo -> hi
            let s = self.ev(arg(a, 0)?, false)?;
            let lo = self.ev(arg(a, 1)?, false)?;
            let hi = self.ev(arg(a, 2)?, false)?;
            let (p, d);
            if self.verified {
                d = self.op(format!("sub i64 {}, {}", hi.a, lo.a))?;
                let g = self.op(format!("getelementptr inbounds i8, ptr {}, i64 {}", s.a, lo.a))?;
                p = self.op(format!("call ptr @pruv_copy_v2(ptr {}, i64 {}, i64 1, i64 1)", g, d))?;
            } else {
                p = self.op(format!("call ptr @pruv_substr(ptr {}, i64 {}, i64 {}, i64 {})", s.a, s.b, lo.a, hi.a))?;
                d = self.op(format!("sub i64 {}, {}", hi.a, lo.a))?;
            }
            let mut rv = V::with_b(Kind::Arr, p, d);
            rv.ty = Some(STR.to_string());
            return self.temp_arr(rv, owned);
        }
        let funcs = self.funcs;
        let f = match funcs.get(n) {
            Some(f) => *f,
            None => return Err(format!("llvmgen: unsupported call in code: {}", n)),
        };
        if f.foreign.is_some() {
            // G6 A3: trusted rust fn through its bridge adapter
            return self.trusted_call(f, e, owned);
        }
        let mut args: Vec<String> = Vec::new();
        for (i, x) in a.iter().enumerate() {
            let prm = f.params.get(i).ok_or_else(|| IDX_ERR.to_string())?;
            if let Expr::InoutArg(ix) = x {
                if self.is_ext(&prm.ty) || inout_slot_param(prm, self.m.resize_params.contains(&(f.name.clone(), i))) {
                    let p = self.inout_place_ptr(&ix.expr)?;
                    args.push(format!("ptr {}", p));
                } else {
                    // The caller's array pointer and length stay borrowed.
                    let v = self.ev(&ix.expr, false)?;
                    args.push(format!("ptr {}", v.a));
                    if v.k != Kind::Coll {
                        // G-LG7-1: an inout Vec passes only its opaque handle
                        args.push(format!("i64 {}", v.b));
                    }
                }
                continue;
            }
            if self.is_ext(&prm.ty) && prm.take {
                // LG9: the callee owns (and drops) fresh storage holding an owned value
                let v = self.ev(x, true)?;
                let sp = self.spill(&v)?;
                args.push(format!("ptr {}", sp));
                continue;
            }
            if self.is_ext(&prm.ty) {
                // G9 A3: a read argument is a pointer to valid caller-owned storage
                let vp = self.view_ptr(x)?;
                args.push(format!("ptr {}", vp));
                continue;
            }
            let take = prm.take;
            let v = self.ev(x, take)?;
            match v.k {
                Kind::Coll => args.push(format!("ptr {}", v.a)), // G6 A3: opaque handle
                Kind::Arr => {
                    args.push(format!("ptr {}", v.a));
                    args.push(format!("i64 {}", v.b));
                    if take && is_array_name(&prm.ty) {
                        args.push(format!("i64 {}", capof(&v)));
                    }
                }
                Kind::Opt => {
                    let g = self.agg(&v)?;
                    args.push(format!("{} {}", OPT, g));
                }
                Kind::Agg => {
                    let ty = need_ty(&v.ty)?;
                    args.push(format!("{} {}", lty(ty), v.a));
                }
                k => args.push(format!("{} {}", k.name(), v.a)),
            }
        }
        if self.cost { self.cost_call(&f.name, &args)?; }
        let callee = if self.m.explicit {
            // G4 A3 (docs/G4_SPEC.md 4): verified calls go to the body (requires proved); safe-mode calls go through the
            // checked wrapper only when the callee has a requires (no wrapper exists otherwise)
            format!("@{}__pruv_{}", mangle(&f.name), if !self.verified && !f.requires.is_empty() { "entry" } else { "body" })
        } else {
            format!("@{}__pruv_{}", mangle(&f.name), if self.verified { "body" } else { "entry" })
        };
        if self.is_ext(&f.ret) {
            // G9 A3: explicit output storage; the result is a fresh owned value
            let outp = self.new_out(&f.ret);
            let mut all: Vec<String> = vec![format!("ptr {}", outp)];
            all.extend(args.iter().cloned());
            self.ins(format!("call void {}({})", callee, all.join(", ")))?;
            let r = self.op(format!("load {}, ptr {}", lty(&f.ret), outp))?;
            return self.fresh(V::agg_p(r, &f.ret, outp), owned);
        }
        let argl = args.join(", ");
        if f.ret == "unit" {
            self.ins(format!("call void {}({})", callee, argl))?;
            return Ok(V::unit());
        }
        if view_ret(f) {
            // R-6g9-1: a borrowed view of the caller's sources: not a temporary, never freed
            let r = self.op(format!("call {VIEW} {}({})", callee, argl))?;
            let x0 = self.op(format!("extractvalue {VIEW} {}, 0", r))?;
            let x1 = self.op(format!("extractvalue {VIEW} {}, 1", r))?;
            let mut rv = V::with_b(Kind::Arr, x0, x1);
            rv.ty = elem_ty(&f.ret);
            rv.layout = self.m.storage.layout_of(f);
            rv.rng = self.m.storage.range_of(f);
            return Ok(rv);
        }
        let r = self.op(format!("call {} {}({})", lty(&f.ret), callee, argl))?;
        if f.ret == "i64" || f.ret == "bool" {
            return Ok(V::new(vkind(&f.ret), r));
        }
        if is_collection(&f.ret) {
            // G6 A3: fresh owned handle; a temporary unless the caller takes it
            let rv = V { layout: "i64", rng: None, k: Kind::Coll, a: r, b: "None".to_string(), ty: Some(f.ret.clone()), p: None, mv: None, c: None };
            return self.temp_arr(rv, owned);
        }
        if is_agg(&f.ret) {
            return Ok(V::agg(r, &f.ret));
        }
        let x0 = self.op(format!("extractvalue {} {}, 0", lty(&f.ret), r))?;
        let x1 = self.op(format!("extractvalue {} {}, 1", lty(&f.ret), r))?;
        if is_arr_name(&f.ret) {
            let x2 = self.op(format!("extractvalue {} {}, 2", lty(&f.ret), r))?; // P3: the cap travels with the returned buffer
            let mut rv = V::with_b(Kind::Arr, x0, x1);
            rv.c = Some(x2);
            rv.layout = self.m.storage.layout_of(f);
            rv.rng = self.m.storage.range_of(f);
            if let Some(t) = elem_ty(&f.ret) {
                rv.ty = Some(t); // G2 A5
            }
            return self.temp_arr(rv, owned);
        }
        Ok(V::with_b(Kind::Opt, x0, x1))
    }

    // ---------- G6 A3: catalog operations and trusted rust fn through the bridge ----------
    /// Bridge arguments (type, value) of an evaluated value: handle ptr, bool as i64 0/1, (ptr, len), i64.
    fn g6_arg(&mut self, v: &V) -> R<Vec<(String, String)>> {
        if v.k == Kind::Coll {
            return Ok(vec![("ptr".to_string(), v.a.clone())]);
        }
        if v.k == Kind::I1 {
            let z = self.op(format!("zext i1 {} to i64", v.a))?;
            return Ok(vec![("i64".to_string(), z)]);
        }
        if v.k == Kind::Arr {
            return Ok(vec![("ptr".to_string(), v.a.clone()), ("i64".to_string(), v.b.clone())]);
        }
        Ok(vec![("i64".to_string(), v.a.clone())])
    }

    /// Fresh entry-block out slot of the given llvm type; returns its name.
    fn g6_out(&mut self, kind: &str) -> String {
        let base = format!("%g{}", self.nslot);
        self.nslot += 1;
        self.allocas.push(format!("{} = alloca {}", base, kind));
        base
    }

    fn native_vec_call(&mut self, e: &Call, elem: &str, owned: bool) -> R<V> {
        match e.name.as_str() {
            "vec_new" => Ok(V::agg("zeroinitializer", need_ty(&e.ty)?)),
            "vec_push" => {
                let v = self.ev(arg(&e.args, 0)?, true)?;
                // The moved vector is owned while the element is evaluated, so a failure there drops it.
                let pending = self.temps.last().map_or(0, |t| t.len());
                self.temps.last_mut().ok_or_else(|| IDX_ERR.to_string())?.push(v.clone());
                let x = self.ev(arg(&e.args, 1)?, true)?;
                self.temps.last_mut().ok_or_else(|| IDX_ERR.to_string())?.remove(pending);
                let xp = self.spill(&x)?;
                let vp = self.spill(&v)?;
                let ty = need_ty(&e.ty)?;
                self.m.vec_elems.insert(elem.to_string());
                self.ins(format!("call void @pruv_vpush__{elem}(ptr {vp}, ptr {xp})"))?;
                let value = self.op(format!("load {ARR}, ptr {vp}"))?;
                self.fresh(V::agg_p(value, ty, vp), owned)
            }
            "vec_get" => {
                let v = self.ev(arg(&e.args, 0)?, false)?;
                let i = self.ev(arg(&e.args, 1)?, false)?;
                if self.m.vec_get_checked {
                    let len = self.op(format!("extractvalue {ARR} {}, 1", v.a))?;
                    let ok = self.op(format!("icmp ult i64 {}, {len}", i.a))?;
                    self.guard(&ok, Fail::VecGet, false)?;
                }
                let data = self.op(format!("extractvalue {ARR} {}, 0", v.a))?;
                let lt = lty(elem);
                let p = self.op(format!("getelementptr inbounds {lt}, ptr {data}, i64 {}", i.a))?;
                let value = self.op(format!("load {lt}, ptr {p}"))?;
                Ok(V::agg_p(value, elem, p))
            }
            _ => Err(IDX_ERR.to_string()),
        }
    }

    fn native_vec_match(&mut self, m: &Match) -> R<()> {
        let Expr::Call(call) = &m.scrutinee else { return Err(IDX_ERR.to_string()) };
        let elem = native_vec_call_elem(call).ok_or_else(|| IDX_ERR.to_string())?;
        let Expr::InoutArg(place) = arg(&call.args, 0)? else { return Err(IDX_ERR.to_string()) };
        self.begin_temps();
        let vec = self.ev(&place.expr, false)?;
        let index = self.ev(arg(&call.args, 1)?, false)?;
        self.end_temps()?;
        let len = self.op(format!("extractvalue {ARR} {}, 1", vec.a))?;
        let ok = self.op(format!("icmp ult i64 {}, {len}", index.a))?;
        let some = self.label("armT");
        let none = self.label("armF");
        let end = self.label("endmatch");
        self.condbr(&ok, &some, &none)?;
        let mut reach = false;
        for arm in &m.arms {
            let ctor = arm.ctor.as_str();
            self.place(if ctor == "Some" { &some } else { &none });
            let mut bindings = Vec::new();
            if ctor == "Some" {
                if let Some(name) = arm.binders.first() {
                    let data = self.op(format!("extractvalue {ARR} {}, 0", vec.a))?;
                    let p = self.op(format!("getelementptr inbounds {}, ptr {data}, i64 {}", lty(elem), index.a))?;
                    bindings.push((name.clone(), Bind::Slot(V::agg(p, elem))));
                }
            }
            if self.run_block(&arm.body, bindings)? {
                self.br(&end)?;
                reach = true;
            }
        }
        if reach {
            self.place(&end);
        } else {
            self.dead = true;
        }
        Ok(())
    }

    fn catalog_call(&mut self, e: &Call, owned: bool) -> R<V> {
        let n = e.name.as_str();
        let cat = crate::foreign::catalog_fn(n).ok_or_else(|| format!("'{}'", n))?;
        if let Some(elem) = native_vec_call_elem(e) {
            return self.native_vec_call(e, elem, owned);
        }
        let (_low, src) = crate::foreign_shim::inst_types(e);
        let sym = crate::foreign_shim::adapter_sym(&crate::foreign_shim::helper_name(n, &src)?);
        let mut args: Vec<(String, String)> = Vec::new();
        // arguments once, in source order; take arguments are owned values
        for (p, x) in cat.params.iter().zip(e.args.iter()) {
            let x = match x {
                Expr::InoutArg(place) => {
                    if p.inout && p.ty == crate::gtype::GType::i64() {
                        args.push(("ptr".to_string(), self.inout_place_ptr(&place.expr)?));
                        continue;
                    }
                    &*place.expr
                }
                _ => x,
            };
            let v = self.ev(x, p.take)?;
            args.extend(self.g6_arg(&v)?);
        }
        if n == "vec_reserve" {
            self.g6_call(&sym, "void", &args)?;
            return Ok(V::unit());
        }
        if matches!(
            n,
            "vec_new" | "map_new" | "string_from" | "string_new" | "vec_push" | "map_insert" | "string_push_str" | "string_push_char"
                | "file_open_read" | "file_create" | "file_read_all" | "file_write_str" | "file_write_bytes"
                | "proc_args_new" | "proc_args_push" | "proc_spawn" | "proc_spawn_env" | "proc_write_str" | "proc_write_bytes"
                | "proc_close_stdin" | "proc_read" | "proc_read_all"
                | "net_connect" | "net_listen" | "net_accept" | "net_write_str" | "net_write_bytes"
                | "net_shutdown_write" | "net_set_timeout" | "net_read" | "net_read_all" | "ev_new" | "ev_add_socket" | "ev_add_listener" | "ev_add_process" | "ev_add_timer" | "ev_wait" | "ev_read" | "ev_read_into" | "ev_proc_read_into" | "ev_write" | "ev_shutdown_write" | "ev_accept" | "ev_proc_read" | "ev_proc_write" | "ev_proc_close_stdin" | "ev_proc_wait" | "ev_proc_wait_for" | "ev_proc_write_for" | "ev_remove"
        ) {
            // a fresh box, or the very same box pointer of a consuming update
            let r = self.g6_call(&sym, "ptr", &args)?;
            let rv = V { layout: "i64", rng: None, k: Kind::Coll, a: r, b: "None".to_string(), ty: e.ty.clone(), p: None, mv: None, c: None };
            return self.temp_arr(rv, owned);
        }
        if matches!(
            n,
            "vec_get" | "string_byte" | "file_close" | "file_ok" | "file_readable" | "file_writable" | "file_size" | "file_byte"
                | "proc_args_len" | "proc_ok" | "proc_error" | "proc_stdin_open" | "proc_eof" | "proc_size" | "proc_byte"
                | "net_ok" | "net_error" | "net_write_open" | "net_eof" | "net_size" | "net_byte"
                | "net_listener_ok" | "net_listener_error" | "net_listener_port" | "ev_ok" | "ev_error" | "ev_cap" | "ev_count" | "ev_last" | "ev_kind" | "ev_eof" | "ev_write_open" | "ev_event_slot" | "ev_event_kind" | "ev_size" | "ev_byte" | "ev_exit"
        ) {
            let r = self.g6_call(&sym, "i64", &args)?;
            if e.ty.as_deref() == Some("bool") {
                let c = self.op(format!("icmp ne i64 {}, 0", r))?;
                return Ok(V::new(Kind::I1, c));
            }
            return Ok(V::new(Kind::I64, r));
        }
        if n == "map_get" {
            let tag = self.g6_out("i64");
            let val = self.g6_out("i64");
            let mut a2 = args.clone();
            a2.push(("ptr".to_string(), tag.clone()));
            a2.push(("ptr".to_string(), val.clone()));
            self.g6_call(&sym, "void", &a2)?;
            let t = self.op(format!("load i64, ptr {}", tag))?;
            let x = self.op(format!("load i64, ptr {}", val))?;
            let t1 = self.op(format!("icmp ne i64 {}, 0", t))?;
            if e.ty.as_deref() == Some("option") {
                return Ok(V::with_b(Kind::Opt, t1, x));
            }
            let x1 = self.op(format!("icmp ne i64 {}, 0", x))?; // Option<bool>: { i1, i1 }
            let ety = need_ty(&e.ty)?.to_string();
            let tt_ = st(&ety, false);
            let a0 = self.op(format!("insertvalue {} zeroinitializer, i1 {}, 0", tt_, t1))?;
            let a1 = self.op(format!("insertvalue {} {}, i1 {}, 1", tt_, a0, x1))?;
            return Ok(V::agg(a1, &ety));
        }
        if matches!(n, "proc_wait" | "proc_kill" | "net_close" | "net_listener_close" | "ev_close") {
            // G10 proc / net A3: consume the box; (tag 1 = Ok, value) -> Result<i64>
            let tag = self.g6_out("i64");
            let val = self.g6_out("i64");
            let mut a2 = args.clone();
            a2.push(("ptr".to_string(), tag.clone()));
            a2.push(("ptr".to_string(), val.clone()));
            self.g6_call(&sym, "void", &a2)?;
            let t = self.op(format!("load i64, ptr {}", tag))?;
            let x = self.op(format!("load i64, ptr {}", val))?;
            let t1 = self.op(format!("icmp ne i64 {}, 0", t))?;
            return Ok(V::with_b(Kind::Opt, t1, x));
        }
        if n == "proc_bytes" || n == "net_bytes" || n == "ev_bytes" {
            // G10 proc / net A3: a borrowed view of the public buffer, copied at once into a fresh [u8]
            self.m.g6_v2 = true;
            let d = self.g6_out("ptr");
            let l = self.g6_out("i64");
            let mut a2 = args.clone();
            a2.push(("ptr".to_string(), d.clone()));
            a2.push(("ptr".to_string(), l.clone()));
            self.g6_call(&sym, "void", &a2)?;
            let dp = self.op(format!("load ptr, ptr {}", d))?;
            let ln = self.op(format!("load i64, ptr {}", l))?;
            let p = self.op(format!("call ptr @pruv_copy_v2(ptr {}, i64 {}, i64 1, i64 1)", dp, ln))?;
            let mut rv = V::with_b(Kind::Arr, p, ln);
            rv.ty = Some(ARRAY_U8.to_string());
            return self.temp_arr(rv, owned);
        }
        if n == "string_to_str" {
            self.m.g6_v2 = true;
            let d = self.g6_out("ptr");
            let l = self.g6_out("i64");
            let mut a2 = args.clone();
            a2.push(("ptr".to_string(), d.clone()));
            a2.push(("ptr".to_string(), l.clone()));
            // P3 M3: take the buffer of a dying owned local String instead of copying it
            let mut mode = "";
            if let (Some(Expr::Var(x)), false) = (e.args.first(), self.cost) {
                if self.ret_take.contains(&x.name) {
                    mode = "ret";
                } else if self.take_let.contains(&x.name) {
                    mode = "let";
                }
            }
            if !mode.is_empty() {
                self.m.g6_take = true;
                let c = self.g6_out("i64"); // P3: the String's capacity travels as the cap (no shrinking realloc)
                a2.push(("ptr".to_string(), c.clone()));
                self.g6_call(crate::foreign_shim::STRING_TAKE_SYM, "void", &a2)?;
                let name = match e.args.first() {
                    Some(Expr::Var(x)) => x.name.clone(),
                    _ => return Err(IDX_ERR.to_string()),
                };
                let sl = self.lookup(&name)?.val().clone();
                if mode == "ret" {
                    self.ret_moved.insert(sl.a.clone(), None); // the return's cleanup skips the slot
                } else {
                    self.ins(format!("store ptr null, ptr {}", sl.a))?; // the scope cleanup drops a null handle as nothing
                }
                let dp = self.op(format!("load ptr, ptr {}", d))?;
                let ln = self.op(format!("load i64, ptr {}", l))?;
                let cp = self.op(format!("load i64, ptr {}", c))?;
                let mut rv = V::with_b(Kind::Arr, dp, ln);
                rv.ty = Some(STR.to_string());
                rv.c = Some(cp);
                return self.temp_arr(rv, owned);
            }
            self.g6_call(crate::foreign_shim::STRING_BYTES_SYM, "void", &a2)?;
            let dp = self.op(format!("load ptr, ptr {}", d))?;
            let ln = self.op(format!("load i64, ptr {}", l))?;
            let p = self.op(format!("call ptr @pruv_copy_v2(ptr {}, i64 {}, i64 1, i64 1)", dp, ln))?;
            let mut rv = V::with_b(Kind::Arr, p, ln);
            rv.ty = Some(STR.to_string());
            return self.temp_arr(rv, owned);
        }
        Err(format!("llvmgen: unsupported catalog operation {}", n))
    }

    fn trusted_call(&mut self, f: &Function, e: &Call, owned: bool) -> R<V> {
        let mut args: Vec<(String, String)> = Vec::new();
        for x in &e.args {
            let v = self.ev(x, false)?;
            args.extend(self.g6_arg(&v)?);
        }
        let sym = crate::foreign_shim::adapter_sym(&f.name);
        if is_collection(&f.ret) {
            // G6b: a fresh boxed result; a temporary unless the caller takes it
            let r = self.g6_call(&sym, "ptr", &args)?;
            let rv = V { layout: "i64", rng: None, k: Kind::Coll, a: r, b: "None".to_string(), ty: Some(f.ret.clone()), p: None, mv: None, c: None };
            return self.temp_arr(rv, owned);
        }
        if f.ret == "unit" {
            self.g6_call(&sym, "void", &args)?;
            return Ok(V::unit());
        }
        let r = self.g6_call(&sym, "i64", &args)?;
        if f.ret == "bool" {
            let c = self.op(format!("icmp ne i64 {}, 0", r))?;
            return Ok(V::new(Kind::I1, c));
        }
        Ok(V::new(Kind::I64, r))
    }

    // ---------- contract (requires) evaluation, i128 ----------
    pub fn bind_contract_params(&mut self) -> R<()> {
        self.place("start");
        let f = self.f;
        self.cparams = f.params.iter().map(|p| p.name.clone()).collect();
        for (i, p) in f.params.iter().enumerate() {
            let n = mangle(&p.name);
            if p.mode_pos.is_some() && is_array_name(&p.ty) && self.m.resize_params.contains(&(f.name.clone(), i)) {
                let slot = self.array_slot(format!("%p.{n}"), &p.ty)?;
                let v = self.load(&slot)?;
                self.cenv.insert(p.name.clone(), v);
            } else if p.mode_pos.is_some() && matches!(p.ty.as_str(), "bool" | "i64") {
                let v = self.load(&V::new(vkind(&p.ty), format!("%p.{n}")))?;
                let v = if v.k == Kind::I64 {
                    V::new(Kind::I128, self.op(format!("sext i64 {} to i128", v.a))?)
                } else {
                    v
                };
                self.cenv.insert(p.name.clone(), v);
            } else if is_arr_name(&p.ty) {
                let mut av = V::with_b(Kind::Arr, format!("%p.{}.a", n), format!("%p.{}.n", n));
                av.layout = self.m.storage.layout_of(p);
                av.rng = self.m.storage.range_of(p);
                if let Some(t) = elem_ty(&p.ty) {
                    av.ty = Some(t); // G2 A5
                }
                self.cenv.insert(p.name.clone(), av);
            } else if p.ty == "bool" {
                self.cenv.insert(p.name.clone(), V::new(Kind::I1, format!("%p.{}", n)));
            } else if p.ty == "i64" {
                let r = self.op(format!("sext i64 %p.{} to i128", n))?;
                self.cenv.insert(p.name.clone(), V::new(Kind::I128, r));
            } else if is_collection(&p.ty) {
                // G6 A3: only len(c) is evaluable in a requires on LLVM
                self.cenv.insert(
                    p.name.clone(),
                    V { layout: "i64", rng: None, k: Kind::Coll, a: format!("%p.{}.h", n), b: "None".to_string(), ty: Some(p.ty.clone()), p: None, mv: None, c: None },
                );
            } else if self.is_ext(&p.ty) {
                // G9 A3: no i128 mirror; contracts read the REAL storage lazily (see cx)
                self.cenv.insert(
                    p.name.clone(),
                    V { layout: "i64", rng: None, k: Kind::Ref, a: format!("%p.{}", n), b: "None".to_string(), ty: Some(p.ty.clone()), p: None, mv: None, c: None },
                );
            } else if is_agg(&p.ty) {
                let param = format!("%p.{}", n);
                let v = if p.mode_pos.is_some() {
                    self.load(&V::agg(param, &p.ty))?
                } else {
                    V::agg(param, &p.ty)
                };
                let c = self.to_c(&v, &p.ty)?;
                self.cenv.insert(p.name.clone(), c);
            } else {
                let t = self.op(format!("extractvalue {} %p.{}, 0", OPT, n))?;
                let v = self.op(format!("extractvalue {} %p.{}, 1", OPT, n))?;
                let w = self.op(format!("sext i64 {} to i128", v))?;
                self.cenv.insert(p.name.clone(), V::with_b(Kind::Opt, t, w));
            }
        }
        Ok(())
    }

    // ---------- G9 A3: contract operands over REAL values ----------
    fn tag_ptr(&mut self, ty: &str, p: &str) -> R<String> {
        self.op(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 0", lty(ty), p))
    }

    /// Pointer to the REAL (code form) storage of a contract operand of nominal or buffer type. Nothing is converted
    /// eagerly, so an inactive variant or a skipped operand is never inspected. Supported (same set as the Rust
    /// backend): a parameter, a field path, variant_field (partial: pruv_panic_undefined when the variant is not active;
    /// the tag is checked before the payload address is formed) and nullary constructors.
    fn cx(&mut self, e: &Expr) -> R<String> {
        let tt = self.tt;
        match e {
            Expr::Var(x) => {
                if !self.cparams.contains(&x.name) {
                    return Err(g9_diag_err(x.line, x.col, &format!("the variable {} as an aggregate operand", x.name)));
                }
                let n = mangle(&x.name);
                if self.is_ext_opt(&x.ty) || self.f.params.iter().any(|p| p.name == x.name && p.mode_pos.is_some() && is_agg(&p.ty)) {
                    return Ok(format!("%p.{}", n));
                }
                // a non-extended parameter is an SSA value
                let ty = need_ty(&x.ty)?.to_string();
                self.spill(&V::agg(format!("%p.{}", n), &ty))
            }
            Expr::FieldAccess(x) => {
                let bty = need_ty(expr_ty(&x.base))?.to_string();
                let bp = self.cx(&x.base)?;
                let i = t_field_index(tt, &bty, &x.field, None)?;
                let g = self.op(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 {}", lty(&bty), bp, i))?;
                if self.fb(&bty, None, &x.field) {
                    return self.op(format!("load ptr, ptr {}", g));
                }
                Ok(g)
            }
            Expr::VariantField(x) => {
                let ety = x.enum_name.clone();
                let bp = self.cx(&x.value)?;
                let k = t_variant_index(tt, &ety, &x.variant)?;
                let tp = self.tag_ptr(&ety, &bp)?;
                let tag = self.op(format!("load i64, ptr {}", tp))?;
                let ok = self.op(format!("icmp eq i64 {}, {}", tag, k))?;
                self.guard(
                    &ok,
                    Fail::Panic1(
                        "pruv_panic_undefined",
                        format!(
                            "variant_field expected {}::{}.{}",
                            crate::emit_common::src_name(&x.enum_name),
                            x.variant,
                            x.field
                        ),
                    ),
                    false,
                )?;
                let i = t_field_index(tt, &ety, &x.field, Some(&x.variant))?;
                let g = g9_field_gep(self, &ety, &bp, Some(&x.variant), i)?;
                if self.fb(&ety, Some(&x.variant), &x.field) {
                    return self.op(format!("load ptr, ptr {}", g));
                }
                Ok(g)
            }
            Expr::EnumCtor(x) if x.args.is_empty() => {
                let ety = x.enum_name.clone();
                let k = t_variant_index(tt, &ety, &x.variant)?;
                let v = self.op(format!("insertvalue {} zeroinitializer, i64 {}, 0", st(&ety, false), k))?;
                self.spill(&V::agg(v, &ety))
            }
            Expr::StructLit(x) if x.fields.is_empty() => self.spill(&V::agg("zeroinitializer", &x.name)),
            Expr::EnumCtor(_) | Expr::StructLit(_) => {
                // A constructor with payload is a REAL temporary of the type (code form) in entry-block storage. An
                // extended one owns heap parts: it is dropped when the enclosing evaluation region ends (cregion).
                let v = self.g9_real(e, None)?;
                let ty = need_ty(&v.ty)?.to_string();
                let base = format!("%ct{}", self.nslot);
                self.nslot += 1;
                self.allocas.push(format!("{} = alloca {}", base, lty(&ty)));
                self.ins(format!("store {} {}, ptr {}", lty(&ty), v.a, base))?;
                if self.is_ext(&ty) {
                    self.cregions.last_mut().ok_or_else(|| IDX_ERR.to_string())?.push((ty, base.clone()));
                }
                Ok(base)
            }
            _ => Err(g9_diag_err(e.line(), e.col(), "this expression as an aggregate operand")),
        }
    }

    /// Runs f (a contract evaluation) and then drops the extended constructor temporaries it created, newest first.
    /// A region is a requires clause, a short-circuit right side or a quantifier body iteration: every temporary of a
    /// region is built on the straight-line path that reaches its end.
    fn cregion(&mut self, f: &dyn Fn(&mut FnGen<'a>) -> R<V>) -> R<V> {
        self.cregions.push(Vec::new());
        let r = f(self)?;
        let ts = self.cregions.pop().ok_or_else(|| "pop from empty list".to_string())?;
        for (ty, p) in ts.iter().rev() {
            self.ins(format!("call void @pruv_drop__{}(ptr {})", ty, p))?;
        }
        Ok(r)
    }

    /// V (code form, OWNED) of the REAL value of the contract expression e of declared type ty (default: its own type).
    /// Constructors are built field by field in declaration order; an i64 comes from the i128 contract value through a
    /// checked narrowing; a named value / part is charged its weight and then deep-copied; a buffer is charged its
    /// length and then copied. Same order of effects and messages as the Rust backend.
    fn g9_real(&mut self, e: &Expr, ty: Option<&str>) -> R<V> {
        let tt = self.tt;
        match e {
            Expr::StructLit(x) => {
                let sty = x.name.as_str();
                let mut inits: HashMap<&str, &Expr> = HashMap::new();
                for f in &x.fields {
                    inits.insert(f.name.as_str(), &f.value);
                }
                let mut acc = "zeroinitializer".to_string();
                for (i, fd) in t_fields(tt, sty, None)?.iter().enumerate() {
                    let init = inits.get(fd.name.as_str()).copied().ok_or_else(|| format!("'{}'", fd.name))?;
                    let v = self.g9_real(init, Some(&fd.ty))?;
                    acc = self.putf(&acc, sty, i, &v, fd, None, false)?;
                }
                return Ok(V::agg(acc, sty));
            }
            Expr::EnumCtor(x) => {
                let ety = x.enum_name.as_str();
                let tag = t_variant_index(tt, ety, &x.variant)?;
                let variant = (!x.args.is_empty()).then_some(x.variant.as_str());
                let t = variant_type(tt, ety, variant, false);
                let mut acc = self.op(format!("insertvalue {} zeroinitializer, i64 {}, 0", t, tag))?;
                for (i, (a, fd)) in x.args.iter().zip(t_fields(tt, ety, Some(&x.variant))?.iter()).enumerate() {
                    let v = self.g9_real(a, Some(&fd.ty))?;
                    acc = self.putf(&acc, ety, i, &v, fd, Some(&x.variant), false)?;
                }
                return self.finish_enum(acc, ety, variant, false);
            }
            _ => {}
        }
        let t: Option<String> = match ty {
            Some(t) => Some(t.to_string()),
            None => expr_ty(e).clone(),
        };
        if let Expr::Ctor(x) = e {
            let px: String = match t.as_deref() {
                Some(t) if is_gopt(t) => g_arg(t).to_string(),
                _ => "i64".to_string(),
            };
            if let Some(gt) = t.as_deref().filter(|s| is_gopt(s)) {
                if x.name == "None" {
                    return Ok(V::agg("zeroinitializer", gt));
                }
                if x.arg.is_none() && x.name == "Ok" {
                    // LB8 R-6g7-1: Ok(())
                    let r = self.op(format!("insertvalue {} zeroinitializer, i1 true, 0", st(gt, false)))?;
                    return Ok(V::agg(r, gt));
                }
                let a = x.arg.as_ref().ok_or_else(|| "llvmgen: constructor without argument".to_string())?;
                if x.name == "Some" || x.name == "Ok" {
                    let xv = self.g9_real(a, Some(&px))?;
                    let acc = self.op(format!("insertvalue {} zeroinitializer, i1 true, 0", st(gt, false)))?;
                    let r = self.put(&acc, gt, 1, &xv, &px, false)?;
                    return Ok(V::agg(r, gt));
                }
                let et = g_error(gt);
                let xv = self.g9_real(a, Some(et))?;
                let r = self.put("zeroinitializer", gt, 2, &xv, et, false)?;
                return Ok(V::agg(r, gt));
            }
            if x.name == "None" {
                return Ok(V::with_b(Kind::Opt, "false", "0"));
            }
            let a = x.arg.as_ref().ok_or_else(|| "llvmgen: constructor without argument".to_string())?;
            let xv = self.g9_real(a, Some(if x.name == "Err" { "i64" } else { px.as_str() }))?;
            return Ok(V::with_b(Kind::Opt, if x.name == "Some" || x.name == "Ok" { "true" } else { "false" }, xv.a));
        }
        if t.is_none() || t.as_deref() == Some("i64") {
            let x = self.cev(e)?;
            let tr = self.op(format!("trunc i128 {} to i64", x.a))?;
            let back = self.op(format!("sext i64 {} to i128", tr))?;
            let ok = self.op(format!("icmp eq i128 {}, {}", back, x.a))?;
            self.guard(&ok, Fail::Panic1("pruv_panic_undefined", "constructor field out of i64 range".to_string()), false)?;
            return Ok(V::new(Kind::I64, tr));
        }
        let t = need_ty(&t)?.to_string();
        if t == "bool" {
            let x = self.cev(e)?;
            return Ok(V::new(Kind::I1, x.a));
        }
        if matches!(e, Expr::Var(_) | Expr::FieldAccess(_) | Expr::VariantField(_))
            && (crate::emit_common::is_user_key(&t) || t == "option" || t == "result" || is_gopt(&t))
        {
            if crate::emit_common::is_user_key(&t) && self.tt.holds_buffer(&t) {
                return Err(g9_diag_err(e.line(), e.col(), "a copy of a value holding an array or str"));
            }
            let p = self.cx(e)?;
            self.g9_charge_weight(&t, &p)?;
            if self.is_ext(&t) {
                return self.clone_val(&V::agg_p("None", &t, p));
            }
            let x = self.op(format!("load {}, ptr {}", lty(&t), p))?;
            return self.unpack(&x, &t, false);
        }
        if is_arr_name(&t) {
            let a = self.cev(e)?;
            self.spend_n(&a.b)?;
            let np = if is_bytes_ty(&a.ty) {
                self.op(format!("call ptr @pruv_copy_v2(ptr {}, i64 {}, i64 1, i64 1)", a.a, a.b))?
            } else {
                self.op(format!("call ptr @pruv_copy(ptr {}, i64 {})", a.a, a.b))?
            };
            let mut r = V::with_b(Kind::Arr, np, a.b.clone());
            r.ty = elem_ty(&t);
            return Ok(r);
        }
        Err(g9_diag_err(e.line(), e.col(), "this expression as a constructor field of an aggregate operand"))
    }

    /// e reads a part of a REAL (extended) value through its storage (cx): a variant_field of an extended value, or a
    /// field path below one.
    fn g9_real_path(&self, e: &Expr) -> bool {
        match e {
            Expr::VariantField(x) => self.is_ext_opt(expr_ty(&x.value)) || self.g9_real_path(&x.value),
            Expr::FieldAccess(x) => self.is_ext_opt(expr_ty(&x.base)) || self.g9_real_path(&x.base),
            _ => false,
        }
    }

    /// Charges the contract budget the weight of the value of type t stored at p (one unit per nominal node); a type
    /// without nominal parts costs nothing.
    fn g9_charge_weight(&mut self, t: &str, p: &str) -> R<()> {
        for n in g9_nominals_in(t) {
            self.m.need_size.insert(n);
        }
        let bud = self.budget.clone().ok_or_else(|| "'FnGen' object has no attribute 'budget'".to_string())?;
        g9_weight(self, t, p, &bud)?;
        Ok(())
    }

    /// Value of a field path / variant_field whose root is an extended value: a buffer (arr), an extended value (ref)
    /// or a scalar-like value converted to its contract form.
    fn cload(&mut self, e: &Expr) -> R<V> {
        let p = self.cx(e)?;
        let t = need_ty(expr_ty(e))?.to_string();
        if is_arr_name(&t) {
            let x = self.op(format!("load {}, ptr {}", ARR, p))?;
            let p0 = self.op(format!("extractvalue {} {}, 0", ARR, x))?;
            let p1 = self.op(format!("extractvalue {} {}, 1", ARR, x))?;
            let mut v = V::with_b(Kind::Arr, p0, p1);
            v.ty = elem_ty(&t);
            return Ok(v);
        }
        if is_collection(&t) {
            let handle = self.op(format!("load ptr, ptr {p}"))?;
            return self.unpack(&handle, &t, false);
        }
        if self.is_ext(&t) {
            return Ok(V { layout: "i64", rng: None, k: Kind::Ref, a: p, b: "None".to_string(), ty: Some(t), p: None, mv: None, c: None });
        }
        // a whole nominal value is converted whole: charge its weight to the budget first
        self.g9_charge_weight(&t, &p)?;
        let x = self.op(format!("load {}, ptr {}", st(&t, false), p))?;
        let u = self.unpack(&x, &t, false)?;
        self.to_c(&u, &t)
    }

    pub fn cev(&mut self, e: &Expr) -> R<V> {
        let und = |msg: String| Fail::Panic1("pruv_panic_undefined", msg);
        let tt = self.tt;
        match e {
            Expr::Spent(_) => Err("cost_unavailable: Stage 5 cost verification is not implemented yet".into()),
            Expr::IntLit(x) => Ok(V::new(Kind::I128, x.value.clone())),
            Expr::BoolLit(x) => Ok(V::new(Kind::I1, if x.value { "true" } else { "false" })),
            Expr::StrLit(x) => {
                // G2 B2: point at the global string directly, without copying
                let g = self.str_global(&x.value);
                Ok(V { layout: "i64", rng: None, k: Kind::Arr, a: g, b: x.value.len().to_string(), ty: x.ty.clone(), p: None, mv: None, c: None })
            }
            Expr::Var(x) => match self.cenv.get(&x.name) {
                Some(v) => Ok(v.clone()),
                None => Err(format!("llvmgen: unsupported variable in requires: {}", x.name)),
            },
            Expr::Unary(x) => {
                let v = self.cev(&x.operand)?;
                if x.op == "!" {
                    let r = self.op(format!("xor i1 {}, true", v.a))?;
                    return Ok(V::new(Kind::I1, r));
                }
                if x.op == "~" {
                    let r = self.op(format!("xor i128 {}, -1", v.a))?;
                    return Ok(V::new(Kind::I128, r));
                }
                let r = self.checked("i128", "ssub", "unary -", "0", &v.a)?;
                Ok(V::new(Kind::I128, r))
            }
            Expr::Binary(x) => {
                let op = x.op.as_str();
                if op == "&&" || op == "||" || op == "==>" {
                    let mut l = self.cev(&x.left)?;
                    if op == "==>" {
                        let r = self.op(format!("xor i1 {}, true", l.a))?;
                        l = V::new(Kind::I1, r);
                    }
                    let right = &x.right;
                    let r = self.shortcircuit(&l.a, op != "&&", &|g| Ok(g.cregion(&|g2| g2.cev(right))?.a))?;
                    return Ok(V::new(Kind::I1, r));
                }
                if (op == "==" || op == "!=") && self.is_ext_opt(expr_ty(&x.left)) {
                    // G9 A3: structural equality of buffer-free extended aggregates, visited lazily on the real values:
                    // one budget unit per visited node pair, only the active variant's fields are compared
                    let pl = self.cx(&x.left)?;
                    let pr = self.cx(&x.right)?;
                    let lt = need_ty(expr_ty(&x.left))?.to_string();
                    self.m.need_eq.insert(lt.clone());
                    let bud = self
                        .budget
                        .clone()
                        .ok_or_else(|| "'FnGen' object has no attribute 'budget'".to_string())?;
                    let mut eq = self.op(format!("call i1 @pruv_ceq__{}(ptr {}, ptr {}, ptr {})", lt, pl, pr, bud))?;
                    if op == "!=" {
                        eq = self.op(format!("xor i1 {}, true", eq))?;
                    }
                    return Ok(V::new(Kind::I1, eq));
                }
                let l = self.cev(&x.left)?;
                let r = self.cev(&x.right)?;
                if let Some(ins) = bit_operator_llvm(op) {
                    let shift = matches!(op, "<<" | ">>");
                    let args = [&l, &r];
                    for arg in &args[..if shift { 1 } else { 2 }] {
                        let biased = self.op(format!("add i128 {}, 9223372036854775808", arg.a))?;
                        let ok = self.op(format!("icmp ult i128 {}, 18446744073709551616", biased))?;
                        self.guard(&ok, Fail::Panic1("pruv_panic_undefined", format!("argument outside i64 in {}", op)), false)?;
                    }
                    if shift {
                        let ok = self.op(format!("icmp ult i128 {}, 64", r.a))?;
                        self.guard(&ok, Fail::Panic1("pruv_panic_undefined", format!("shift count outside 0..64 in {}", op)), false)?;
                    }
                    let v = self.op(format!("{} i128 {}, {}", ins, l.a, r.a))?;
                    if op == "<<" {
                        let biased = self.op(format!("add i128 {}, 9223372036854775808", v))?;
                        let ok = self.op(format!("icmp ult i128 {}, 18446744073709551616", biased))?;
                        self.guard(&ok, Fail::Panic1("pruv_panic_undefined", "overflow in <<".to_string()), false)?;
                    }
                    return Ok(V::new(Kind::I128, v));
                }
                if op == "+" || op == "-" || op == "*" {
                    let name = match op {
                        "+" => "sadd",
                        "-" => "ssub",
                        _ => "smul",
                    };
                    let v = self.checked("i128", name, op, &l.a, &r.a)?;
                    return Ok(V::new(Kind::I128, v));
                }
                if op == "/" || op == "%" {
                    let func = if op == "/" { "pruv_cdiv" } else { "pruv_crem" };
                    let v = self.op(format!("call i128 @{}(i128 {}, i128 {})", func, l.a, r.a))?;
                    return Ok(V::new(Kind::I128, v));
                }
                if (op == "==" || op == "!=") && l.k == Kind::Agg {
                    let lty_ = need_ty(&l.ty)?.to_string();
                    let mut eq = self.ceq(&l, &r, &lty_)?;
                    if op == "!=" {
                        eq = self.op(format!("xor i1 {}, true", eq))?;
                    }
                    return Ok(V::new(Kind::I1, eq));
                }
                if (op == "==" || op == "!=") && l.k == Kind::Opt {
                    let te = self.op(format!("icmp eq i1 {}, {}", l.a, r.a))?;
                    let mut pe = self.op(format!("icmp eq i128 {}, {}", l.b, r.b))?;
                    if expr_ty(&x.left).as_deref() == Some("option") {
                        let nt = self.op(format!("xor i1 {}, true", l.a))?;
                        pe = self.op(format!("or i1 {}, {}", nt, pe))?;
                    }
                    let mut eq = self.op(format!("and i1 {}, {}", te, pe))?;
                    if op == "!=" {
                        eq = self.op(format!("xor i1 {}, true", eq))?;
                    }
                    return Ok(V::new(Kind::I1, eq));
                }
                let cmp = cmp_name(op)?;
                let ty = if l.k == Kind::I1 { "i1" } else { "i128" };
                let v = self.op(format!("icmp {} {} {}, {}", cmp, ty, l.a, r.a))?;
                Ok(V::new(Kind::I1, v))
            }
            Expr::Index(x) if expr_ty(&x.array).as_deref().and_then(native_vec_elem).is_some() => self.cload(e),
            Expr::Index(x) => {
                if expr_ty(&x.array).as_deref().map_or(false, is_collection) {
                    // G6 A3: not evaluable on LLVM yet
                    return Err(g6_diag_err(x.line, x.col));
                }
                let a = self.cev(&x.array)?;
                let i = self.cev(&x.index)?;
                let ln = self.op(format!("zext i64 {} to i128", a.b))?;
                let ok = self.op(format!("icmp ult i128 {}, {}", i.a, ln))?;
                self.guard(&ok, und("array index out of range".to_string()), false)?;
                let ii = self.op(format!("trunc i128 {} to i64", i.a))?;
                if a.ty.as_deref() == Some(ARRAY_BOOL) {
                    // G2 A5
                    let g = self.op(format!("getelementptr inbounds i8, ptr {}, i64 {}", a.a, ii))?;
                    let x8 = self.op(format!("load i8, ptr {}", g))?;
                    let b = self.op(format!("icmp ne i8 {}, 0", x8))?;
                    return Ok(V::new(Kind::I1, b));
                }
                if is_u8_ty(&a.ty) || is_strv(&a.ty) {
                    let g = self.op(format!("getelementptr inbounds i8, ptr {}, i64 {}", a.a, ii))?;
                    let x8 = self.op(format!("load i8, ptr {}", g))?;
                    let r = self.op(format!("zext i8 {} to i128", x8))?;
                    return Ok(V::new(Kind::I128, r));
                }
                if a.layout != "i64" {
                    let g = self.op(format!("getelementptr inbounds {}, ptr {}, i64 {}", elem_ir(&a), a.a, ii))?;
                    return Ok(V::new(Kind::I128, self.narrow_load(&a, &g, "i128")?));
                }
                let g = self.op(format!("getelementptr inbounds i64, ptr {}, i64 {}", a.a, ii))?;
                let v = self.op(format!("load i64, ptr {}", g))?;
                let r = self.op(format!("sext i64 {} to i128", v))?;
                Ok(V::new(Kind::I128, r))
            }
            Expr::Ctor(x) => {
                if x.ty.as_deref().map_or(false, is_gopt) {
                    return self.mk_gctor(x, true); // G3
                }
                if x.name == "None" {
                    return Ok(V::with_b(Kind::Opt, "false", "0"));
                }
                let a = x.arg.as_ref().ok_or_else(|| "llvmgen: constructor without argument".to_string())?;
                let v = self.cev(a)?;
                Ok(V::with_b(Kind::Opt, if x.name == "Some" || x.name == "Ok" { "true" } else { "false" }, v.a))
            }
            Expr::Quant(_) => self.cquant(e),
            Expr::StructLit(x) => self.mk_struct(x, true, false),
            Expr::EnumCtor(x) => self.mk_enum(x, true, false),
            Expr::FieldAccess(x) => {
                if self.is_ext_opt(expr_ty(&x.base)) || self.g9_real_path(&x.base) {
                    // G9: a field of a real value, read through its storage
                    return self.cload(e);
                }
                let b = self.cev(&x.base)?;
                let bty = need_ty(&b.ty)?.to_string();
                let i = t_field_index(tt, &bty, &x.field, None)?;
                let fds = t_fields(tt, &bty, None)?;
                let fty = fds.get(i).ok_or_else(|| IDX_ERR.to_string())?.ty.as_str();
                self.get(&b, &bty, i, fty, true)
            }
            Expr::IsVariant(x) => {
                if self.is_ext_opt(expr_ty(&x.value)) || self.g9_real_path(&x.value) {
                    // G9: the tag of the real value
                    let ety = need_ty(expr_ty(&x.value))?.to_string();
                    let p = self.cx(&x.value)?;
                    let tp = self.tag_ptr(&ety, &p)?;
                    let tag = self.op(format!("load i64, ptr {}", tp))?;
                    let k = t_variant_index(tt, &ety, &x.variant)?;
                    let r = self.op(format!("icmp eq i64 {}, {}", tag, k))?;
                    return Ok(V::new(Kind::I1, r));
                }
                let b = self.cev(&x.value)?;
                let bty = need_ty(&b.ty)?.to_string();
                let k = t_variant_index(tt, &bty, &x.variant)?;
                let tag = self.tag_of(&b, true)?;
                let r = self.op(format!("icmp eq i64 {}, {}", tag, k))?;
                Ok(V::new(Kind::I1, r))
            }
            // G6 A0: `move` is checked but never lowered (the driver's A0 guard stops the program earlier)
            Expr::Move(_) => Err("G6 move reached the LLVM generator".to_string()),
            // G5 A0: an inout marker is checked but never lowered (the driver's A0 guard stops the program earlier)
            Expr::InoutArg(_) => Err("G5 inout argument reached the LLVM generator".to_string()),
            Expr::VariantField(x) => {
                if self.is_ext_opt(expr_ty(&x.value)) || self.g9_real_path(&x.value) {
                    return self.cload(e);
                }
                let b = self.cev(&x.value)?;
                let bty = need_ty(&b.ty)?.to_string();
                let k = t_variant_index(tt, &bty, &x.variant)?;
                let tag = self.tag_of(&b, true)?;
                let ok = self.op(format!("icmp eq i64 {}, {}", tag, k))?;
                self.guard(
                    &ok,
                    und(format!("variant_field expected {}::{}.{}", x.enum_name, x.variant, x.field)),
                    false,
                )?;
                let i = t_field_index(tt, &bty, &x.field, Some(&x.variant))?;
                let fds = t_fields(tt, &bty, Some(&x.variant))?;
                let fd = fds.get(i).ok_or_else(|| IDX_ERR.to_string())?;
                let pi = flattened_payload_index(tt, &bty, &x.variant, i)?;
                self.get(&b, &bty, pi, &fd.ty, true)
            }
            Expr::Call(x) => {
                let n = x.name.as_str();
                let a = &x.args;
                if crate::foreign::FILE_OBSERVERS.contains(&n) {
                    let v = self.cev(arg(a, 0)?)?;
                    let mut args = vec![("ptr".to_string(), v.a)];
                    if n == "file_byte" {
                        let i = self.cev(arg(a, 1)?)?;
                        let size = self.g6_call(&crate::foreign_shim::adapter_sym("pruv_g6_file_size"), "i64", &args)?;
                        let size = self.op(format!("zext i64 {} to i128", size))?;
                        let ok = self.op(format!("icmp ult i128 {}, {}", i.a, size))?;
                        self.guard(&ok, und("array index out of range".to_string()), false)?;
                        args.push(("i64".to_string(), self.op(format!("trunc i128 {} to i64", i.a))?));
                    }
                    let r = self.g6_call(&crate::foreign_shim::adapter_sym(&format!("pruv_g6_{}", n)), "i64", &args)?;
                    if x.ty.as_deref() == Some("bool") {
                        return Ok(V::new(Kind::I1, self.op(format!("icmp ne i64 {}, 0", r))?));
                    }
                    return Ok(V::new(Kind::I128, self.op(format!("zext i64 {} to i128", r))?));
                }
                if crate::foreign::PROC_OBSERVERS.contains(&n) {
                    // G10 proc A3: observers over the public state of the handle; the stream and index guards carry
                    // the undefined-contract messages of the Rust source backend
                    let v = self.cev(arg(a, 0)?)?;
                    let mut args = vec![("ptr".to_string(), v.a)];
                    if matches!(n, "proc_eof" | "proc_size" | "proc_byte") {
                        let st_ = self.cev(arg(a, 1)?)?;
                        let o1 = self.op(format!("icmp eq i128 {}, 1", st_.a))?;
                        let o2 = self.op(format!("icmp eq i128 {}, 2", st_.a))?;
                        let ok = self.op(format!("or i1 {}, {}", o1, o2))?;
                        self.guard(&ok, und("process stream outside 1..2".to_string()), false)?;
                        args.push(("i64".to_string(), self.op(format!("trunc i128 {} to i64", st_.a))?));
                    }
                    if n == "proc_byte" {
                        let i = self.cev(arg(a, 2)?)?;
                        let size = self.g6_call(&crate::foreign_shim::adapter_sym("pruv_g6_proc_size"), "i64", &args)?;
                        let size = self.op(format!("zext i64 {} to i128", size))?;
                        let ok = self.op(format!("icmp ult i128 {}, {}", i.a, size))?;
                        self.guard(&ok, und("process byte index out of range".to_string()), false)?;
                        args.push(("i64".to_string(), self.op(format!("trunc i128 {} to i64", i.a))?));
                    }
                    let r = self.g6_call(&crate::foreign_shim::adapter_sym(&format!("pruv_g6_{}", n)), "i64", &args)?;
                    if x.ty.as_deref() == Some("bool") {
                        return Ok(V::new(Kind::I1, self.op(format!("icmp ne i64 {}, 0", r))?));
                    }
                    if n == "proc_error" {
                        // the error code is 0..6, never negative
                        return Ok(V::new(Kind::I128, self.op(format!("sext i64 {} to i128", r))?));
                    }
                    return Ok(V::new(Kind::I128, self.op(format!("zext i64 {} to i128", r))?));
                }
                if crate::foreign::ASYNC_OBSERVERS.contains(&n) {
                    let v = self.cev(arg(a, 0)?)?;
                    let mut args = vec![("ptr".to_string(), v.a)];
                    if matches!(n, "ev_kind" | "ev_eof" | "ev_write_open" | "ev_byte") {
                        let i = self.cev(arg(a, 1)?)?;
                        let bound = if n == "ev_byte" { "ev_size" } else { "ev_cap" };
                        let size = self.g6_call(&crate::foreign_shim::adapter_sym(&format!("pruv_g6_{}", bound)), "i64", &args)?;
                        let size = self.op(format!("zext i64 {} to i128", size))?;
                        let msg = if n == "ev_byte" { "array index out of range" } else { "loop slot out of range" };
                        let ok = self.op(format!("icmp ult i128 {}, {}", i.a, size))?;
                        self.guard(&ok, und(msg.to_string()), false)?;
                        args.push(("i64".to_string(), self.op(format!("trunc i128 {} to i64", i.a))?));
                    }
                    let r = self.g6_call(&crate::foreign_shim::adapter_sym(&format!("pruv_g6_{}", n)), "i64", &args)?;
                    if x.ty.as_deref() == Some("bool") {
                        return Ok(V::new(Kind::I1, self.op(format!("icmp ne i64 {}, 0", r))?));
                    }
                    return Ok(V::new(Kind::I128, self.op(format!("sext i64 {} to i128", r))?));
                }
                if crate::foreign::NET_OBSERVERS.contains(&n) {
                    // G10 net A3: observers over the public state of the handle; the index guard carries the
                    // undefined-contract message of the Rust source backend
                    let v = self.cev(arg(a, 0)?)?;
                    let mut args = vec![("ptr".to_string(), v.a)];
                    if n == "net_byte" {
                        let i = self.cev(arg(a, 1)?)?;
                        let size = self.g6_call(&crate::foreign_shim::adapter_sym("pruv_g6_net_size"), "i64", &args)?;
                        let size = self.op(format!("zext i64 {} to i128", size))?;
                        let ok = self.op(format!("icmp ult i128 {}, {}", i.a, size))?;
                        self.guard(&ok, und("socket byte index out of range".to_string()), false)?;
                        args.push(("i64".to_string(), self.op(format!("trunc i128 {} to i64", i.a))?));
                    }
                    let r = self.g6_call(&crate::foreign_shim::adapter_sym(&format!("pruv_g6_{}", n)), "i64", &args)?;
                    if x.ty.as_deref() == Some("bool") {
                        return Ok(V::new(Kind::I1, self.op(format!("icmp ne i64 {}, 0", r))?));
                    }
                    if n == "net_error" || n == "net_listener_error" {
                        // the error code is 0..6, never negative
                        return Ok(V::new(Kind::I128, self.op(format!("sext i64 {} to i128", r))?));
                    }
                    return Ok(V::new(Kind::I128, self.op(format!("zext i64 {} to i128", r))?));
                }
                if n == "size" {
                    // G9 A3: structural size of the real value, checked i128 additions, one unit per node
                    let p = self.cx(arg(a, 0)?)?;
                    let aty = need_ty(expr_ty(arg(a, 0)?))?.to_string();
                    self.m.need_size.insert(aty.clone());
                    let bud = self
                        .budget
                        .clone()
                        .ok_or_else(|| "'FnGen' object has no attribute 'budget'".to_string())?;
                    let r = self.op(format!("call i128 @pruv_csize__{}(ptr {}, ptr {})", aty, p, bud))?;
                    return Ok(V::new(Kind::I128, r));
                }
                if n == "len" {
                    let operand = arg(a, 0)?;
                    if expr_ty(operand).as_deref().and_then(native_vec_elem).is_some() {
                        let ptr = self.cx(operand)?;
                        let lp = g9_gep(self, ARR, &ptr, 1)?;
                        let len = self.op(format!("load i64, ptr {lp}"))?;
                        return Ok(V::new(Kind::I128, self.op(format!("zext i64 {len} to i128"))?));
                    }
                    let v = self.cev(operand)?;
                    if v.k == Kind::Coll {
                        // G6 A3
                        let vty = need_ty(&v.ty)?.to_string();
                        let sym = crate::foreign_shim::len_sym(&vty)?;
                        let ln = self.g6_call(&sym, "i64", &[("ptr".to_string(), v.a.clone())])?;
                        let r = self.op(format!("zext i64 {} to i128", ln))?;
                        return Ok(V::new(Kind::I128, r));
                    }
                    let r = self.op(format!("zext i64 {} to i128", v.b))?;
                    return Ok(V::new(Kind::I128, r));
                }
                if crate::foreign::is_catalog_contract_only(n) {
                    // G6 A3: map_lookup / map_updated: not evaluable on LLVM yet
                    return Err(g6_diag_err(x.line, x.col));
                }
                if n == "utf8_valid" {
                    let b = self.cev(arg(a, 0)?)?;
                    self.m.uses_utf8_valid = true;
                    let ok = self.op(format!("call i1 @pruv_utf8_valid(ptr {}, i64 {})", b.a, b.b))?;
                    return Ok(V::new(Kind::I1, ok));
                }
                if n == "utf8_valid_span" {
                    // LG26 (R-6b-3): the --no-verify requires check; false outside the range
                    let b = self.cev(arg(a, 0)?)?;
                    let lo = self.cev(arg(a, 1)?)?;
                    let hi = self.cev(arg(a, 2)?)?;
                    self.m.uses_utf8_valid = true;
                    let ln = self.op(format!("zext i64 {} to i128", b.b))?;
                    let c1 = self.op(format!("icmp sge i128 {}, 0", lo.a))?;
                    let c2 = self.op(format!("icmp sle i128 {}, {}", lo.a, hi.a))?;
                    let c3 = self.op(format!("icmp sle i128 {}, {}", hi.a, ln))?;
                    let c12 = self.op(format!("and i1 {}, {}", c1, c2))?;
                    let ok = self.op(format!("and i1 {}, {}", c12, c3))?;
                    let lt = self.op(format!("trunc i128 {} to i64", lo.a))?;
                    let lo64 = self.op(format!("select i1 {}, i64 {}, i64 0", ok, lt))?;
                    let d = self.op(format!("sub i128 {}, {}", hi.a, lo.a))?;
                    let dt = self.op(format!("trunc i128 {} to i64", d))?;
                    let n64 = self.op(format!("select i1 {}, i64 {}, i64 0", ok, dt))?;
                    let p = self.op(format!("getelementptr i8, ptr {}, i64 {}", b.a, lo64))?;
                    let v = self.op(format!("call i1 @pruv_utf8_valid(ptr {}, i64 {})", p, n64))?;
                    let r = self.op(format!("and i1 {}, {}", ok, v))?;
                    return Ok(V::new(Kind::I1, r));
                }
                if n == "is_char_boundary" {
                    // G2 B2
                    let s = self.cev(arg(a, 0)?)?;
                    let i = self.cev(arg(a, 1)?)?;
                    let ln = self.op(format!("zext i64 {} to i128", s.b))?;
                    let ok = self.op(format!("icmp ule i128 {}, {}", i.a, ln))?;
                    let t = self.op(format!("trunc i128 {} to i64", i.a))?;
                    let u = self.op(format!("select i1 {}, i64 {}, i64 -1", ok, t))?;
                    let r = self.op(format!("call i32 @pruv_str_boundary(ptr {}, i64 {}, i64 {})", s.a, s.b, u))?;
                    let c = self.op(format!("icmp ne i32 {}, 0", r))?;
                    return Ok(V::new(Kind::I1, c));
                }
                if matches!(n, "load_be" | "load_le") {
                    // P2 batch 4: 0 <= i && i + nb <= len(a) in i128, then the word
                    let arr = self.cev(arg(a, 0)?)?;
                    let i = self.cev(arg(a, 1)?)?;
                    let nb = crate::check::word_width(arg(a, 2)?).unwrap_or(0);
                    let ln = self.op(format!("zext i64 {} to i128", arr.b))?;
                    let t = self.op(format!("sub i128 {}, {}", ln, nb))?;
                    let c1 = self.op(format!("icmp sge i128 {}, 0", i.a))?;
                    let c2 = self.op(format!("icmp sle i128 {}, {}", i.a, t))?;
                    let ok = self.op(format!("and i1 {}, {}", c1, c2))?;
                    self.guard(&ok, und("array index out of range".to_string()), false)?;
                    let ii = self.op(format!("trunc i128 {} to i64", i.a))?;
                    let g = self.op(format!("getelementptr inbounds i8, ptr {}, i64 {}", arr.a, ii))?;
                    let w = self.load_word(&g, nb, n == "load_le")?;
                    let r = self.op(format!("sext i64 {} to i128", w))?;
                    return Ok(V::new(Kind::I128, r));
                }
                if let Some(ins) = bit_llvm(n) {
                    // P2: both i128 arguments must be inside i64; then the op on i128 is exact
                    let x = self.cev(arg(a, 0)?)?;
                    let y = self.cev(arg(a, 1)?)?;
                    for v in [&x, &y] {
                        let t = self.op(format!("add i128 {}, 9223372036854775808", v.a))?;
                        let ok = self.op(format!("icmp ult i128 {}, 18446744073709551616", t))?;
                        self.guard(&ok, und(format!("argument outside i64 in {}", n)), false)?;
                    }
                    if n == "wrapping_shl" || n == "ushr" || n == "wrapping_add" || n == "wrapping_mul" {
                        // P2 batch 2/3: shifts and wrapping add/mul are not exact on i128, so truncate to i64, op, widen
                        if n == "wrapping_shl" || n == "ushr" {
                            // count in 0..64 (unsigned compare covers negatives)
                            let ok = self.op(format!("icmp ult i128 {}, 64", y.a))?;
                            self.guard(&ok, und(format!("shift count outside 0..64 in {}", n)), false)?;
                        }
                        let tx = self.op(format!("trunc i128 {} to i64", x.a))?;
                        let ty = self.op(format!("trunc i128 {} to i64", y.a))?;
                        let r = self.op(format!("{} i64 {}, {}", ins, tx, ty))?;
                        let w = self.op(format!("sext i64 {} to i128", r))?;
                        return Ok(V::new(Kind::I128, w));
                    }
                    let r = self.op(format!("{} i128 {}, {}", ins, x.a, y.a))?;
                    return Ok(V::new(Kind::I128, r));
                }
                if matches!(n, "is_some" | "is_ok" | "some_val" | "ok_val" | "err_val")
                    && expr_ty(arg(a, 0)?).as_deref().is_some_and(is_owned_result)
                {
                    // LG15 (G-LG12-4/5): no i128 mirror of the String; the REAL storage { i1, L(T), ptr } is read
                    // lazily through cx (a parameter or a field path), like the Rust backend reads its parameter
                    let a0 = arg(a, 0)?;
                    let oty = expr_ty(a0).clone().unwrap_or_default();
                    let xt = g_arg(&oty).to_string();
                    // LG18 (G-LG15-2): ok_val also of a user struct / enum or a new Option / Result T that is not
                    // extended (its contract mirror comes from to_c); an extended T cannot be a Result payload (type_argument)
                    let okt = matches!(xt.as_str(), "i64" | "bool" | "option" | "result") || is_agg(&xt) && !self.is_ext(&xt);
                    if !matches!(n, "is_ok" | "ok_val" | "err_val") || (n == "ok_val" && !okt) {
                        return Err(g9_diag_err(x.line, x.col, "a Result<_, String> operand"));
                    }
                    let p = self.cx(a0)?;
                    let ot = lty(&oty);
                    let tp = g9_gep(self, &ot, &p, 0)?;
                    let tag = self.op(format!("load i1, ptr {}", tp))?;
                    if n == "is_ok" {
                        return Ok(V::new(Kind::I1, tag));
                    }
                    if n == "ok_val" {
                        self.guard(&tag, und("ok_val of Err".to_string()), false)?;
                        let vp = g9_gep(self, &ot, &p, 1)?;
                        let v = self.op(format!("load {}, ptr {}", lty(&xt), vp))?;
                        if vkind(&xt) == Kind::Opt {
                            let xv = self.unpack(&v, &xt, false)?;
                            return self.to_c(&xv, &xt);
                        }
                        let mut xv = V::new(vkind(&xt), v);
                        xv.ty = Some(xt.clone());
                        return self.to_c(&xv, &xt);
                    }
                    self.guard(&tag, und("err_val of Ok".to_string()), true)?;
                    let hp = g9_gep(self, &ot, &p, 2)?;
                    let h = self.op(format!("load ptr, ptr {}", hp))?;
                    let ety = g_error(&oty).to_string();
                    return Ok(V { layout: "i64", rng: None, k: Kind::Coll, a: h, b: "None".to_string(), ty: Some(ety), p: None, mv: None, c: None });
                }
                if n == "is_some" || n == "is_ok" {
                    let v = self.cev(arg(a, 0)?)?;
                    if v.k == Kind::Agg {
                        // G3: tag of a new Option/Result
                        let ty = need_ty(&v.ty)?.to_string();
                        let r = self.op(format!("extractvalue {} {}, 0", st(&ty, true), v.a))?;
                        return Ok(V::new(Kind::I1, r));
                    }
                    return Ok(V::new(Kind::I1, v.a));
                }
                if n == "some_val" || n == "ok_val" || n == "err_val" {
                    let o = self.cev(arg(a, 0)?)?;
                    if o.k == Kind::Agg {
                        // G3: selectors of a new Option/Result look only at the active payload
                        let oty = need_ty(&o.ty)?.to_string();
                        let tc = st(&oty, true);
                        let tag = self.op(format!("extractvalue {} {}, 0", tc, o.a))?;
                        if n == "err_val" {
                            self.guard(&tag, und("err_val of Ok".to_string()), true)?;
                            return self.get(&o, &oty, 2, g_error(&oty), true);
                        }
                        let msg = if n == "some_val" { "some_val of None" } else { "ok_val of Err" };
                        self.guard(&tag, und(msg.to_string()), false)?;
                        return self.get(&o, &oty, 1, g_arg(&oty), true);
                    }
                    if n == "err_val" {
                        self.guard(&o.a, und("err_val of Ok".to_string()), true)?;
                    } else {
                        let msg = if n == "some_val" { "some_val of None" } else { "ok_val of Err" };
                        self.guard(&o.a, und(msg.to_string()), false)?;
                    }
                    return Ok(V::new(Kind::I128, o.b));
                }
                Err(format!("llvmgen: unsupported expression in requires: {:?}", e))
            }
        }
    }

    /// Consumes 1 unit of the requires iteration budget (docs/M0_CONTRACTS.md B)
    pub fn spend(&mut self) -> R<()> {
        self.spend_n("1")
    }

    /// Consumes n units of the budget. The budget is never negative at rest and n is a non-negative length, so the
    /// subtraction cannot overflow.
    pub fn spend_n(&mut self, n: &str) -> R<()> {
        let budget = match &self.budget {
            Some(b) => b.clone(),
            None => return Err("llvmgen: internal error: no iteration budget slot".to_string()),
        };
        let old = self.op(format!("load i64, ptr {}", budget))?;
        let nv = self.op(format!("sub i64 {}, {}", old, n))?;
        self.ins(format!("store i64 {}, ptr {}", nv, budget))?;
        let ok = self.op(format!("icmp sge i64 {}, 0", nv))?;
        self.guard(&ok, Fail::Panic1("pruv_panic_undefined", "contract iteration budget exceeded".to_string()), false)
    }

    /// The O(n) form of the sorted precondition forall: forall k in 1..len(a): a[k-1] <= a[k]
    pub fn csorted(&mut self, name: &str, q: usize) -> R<V> {
        let arr = self.cenv.get(name).cloned().ok_or_else(|| format!("'{}'", name))?;
        let k = format!("%q{}.k", q);
        let r = format!("%q{}.r", q);
        self.allocas.push(format!("{} = alloca i64", k));
        self.allocas.push(format!("{} = alloca i1", r));
        self.ins(format!("store i64 1, ptr {}", k))?;
        self.ins(format!("store i1 true, ptr {}", r))?;
        let cl = self.label("qc");
        let bl = self.label("qb");
        let nl = self.label("qn");
        let el = self.label("qe");
        self.br(&cl)?;
        self.place(&cl);
        let kv = self.op(format!("load i64, ptr {}", k))?;
        let c = self.op(format!("icmp slt i64 {}, {}", kv, arr.b))?;
        self.condbr(&c, &bl, &el)?;
        self.place(&bl);
        self.spend()?;
        let pk = self.op(format!("sub i64 {}, 1", kv))?;
        let le;
        if arr.layout != "i64" {
            let g0 = self.op(format!("getelementptr inbounds {}, ptr {}, i64 {}", elem_ir(&arr), arr.a, pk))?;
            let z0 = self.narrow_load(&arr, &g0, "i64")?;
            let g1 = self.op(format!("getelementptr inbounds {}, ptr {}, i64 {}", elem_ir(&arr), arr.a, kv))?;
            let z1 = self.narrow_load(&arr, &g1, "i64")?;
            le = self.op(format!("icmp sle i64 {}, {}", z0, z1))?;
        } else if is_u8_ty(&arr.ty) {
            let g0 = self.op(format!("getelementptr inbounds i8, ptr {}, i64 {}", arr.a, pk))?;
            let g1 = self.op(format!("getelementptr inbounds i8, ptr {}, i64 {}", arr.a, kv))?;
            let x0 = self.op(format!("load i8, ptr {}", g0))?;
            let x1 = self.op(format!("load i8, ptr {}", g1))?;
            let z0 = self.op(format!("zext i8 {} to i64", x0))?;
            let z1 = self.op(format!("zext i8 {} to i64", x1))?;
            le = self.op(format!("icmp sle i64 {}, {}", z0, z1))?;
        } else {
            let g0 = self.op(format!("getelementptr inbounds i64, ptr {}, i64 {}", arr.a, pk))?;
            let g1 = self.op(format!("getelementptr inbounds i64, ptr {}, i64 {}", arr.a, kv))?;
            let x0 = self.op(format!("load i64, ptr {}", g0))?;
            let x1 = self.op(format!("load i64, ptr {}", g1))?;
            le = self.op(format!("icmp sle i64 {}, {}", x0, x1))?;
        }
        let stop = self.label("qstop");
        self.condbr(&le, &nl, &stop)?;
        self.place(&stop);
        self.ins(format!("store i1 false, ptr {}", r))?;
        self.br(&el)?;
        self.place(&nl);
        let k2 = self.op(format!("add i64 {}, 1", kv))?;
        self.ins(format!("store i64 {}, ptr {}", k2, k))?;
        self.br(&cl)?;
        self.place(&el);
        let res = self.op(format!("load i1, ptr {}", r))?;
        Ok(V::new(Kind::I1, res))
    }

    /// P3: branch-free prefilter of a fast_forall quantifier (emit_common::fast_forall). An empty range and a range
    /// whose every read is in bounds, that fits the remaining budget and whose body holds everywhere go to el with the
    /// result true (already stored) and the range length spent; everything else falls through to the scalar loop.
    pub fn cfastpre(&mut self, e: &Quant, reads: &[(String, i128)], lo: &str, hi: &str, q: usize, el: &str) -> R<()> {
        let g = self.label("qfg");
        let empty = self.op(format!("icmp sge i128 {}, {}", lo, hi))?;
        self.condbr(&empty, el, &g)?;
        self.place(&g);
        let n = self.op(format!("sub i128 {}, {}", hi, lo))?;
        let budget = match &self.budget {
            Some(b) => b.clone(),
            None => return Err("llvmgen: internal error: no iteration budget slot".to_string()),
        };
        let bl = self.op(format!("load i64, ptr {}", budget))?;
        let bw = self.op(format!("sext i64 {} to i128", bl))?;
        let mut conds = vec![
            self.op(format!("icmp sge i128 {}, -9223372036854775808", lo))?,
            self.op(format!("icmp sle i128 {}, 9223372036854775807", hi))?,
            self.op(format!("icmp sle i128 {}, {}", n, bw))?,
        ];
        for (a, c) in reads {
            let len = self.cenv.get(a).map(|v| v.b.clone()).ok_or_else(|| format!("'{}'", a))?;
            let ln = self.op(format!("zext i64 {} to i128", len))?;
            let lc = if *c == 0 { lo.to_string() } else { self.op(format!("add i128 {}, {}", lo, c))? };
            let hc = if *c == 0 { hi.to_string() } else { self.op(format!("add i128 {}, {}", hi, c))? };
            conds.push(self.op(format!("icmp sge i128 {}, 0", lc))?);
            conds.push(self.op(format!("icmp sle i128 {}, {}", hc, ln))?);
        }
        let mut ok = conds[0].clone();
        for c in &conds[1..] {
            ok = self.op(format!("and i1 {}, {}", ok, c))?;
        }
        let pre = self.label("qfp");
        let slow = self.label("qfs");
        self.condbr(&ok, &pre, &slow)?;
        self.place(&pre);
        let j = format!("%q{}.j", q);
        let acc = format!("%q{}.a", q);
        self.allocas.push(format!("{} = alloca i64", j));
        self.allocas.push(format!("{} = alloca i1", acc));
        let hi64 = self.op(format!("trunc i128 {} to i64", hi))?;
        let lo64 = self.op(format!("trunc i128 {} to i64", lo))?;
        self.ins(format!("store i64 {}, ptr {}", lo64, j))?;
        self.ins(format!("store i1 true, ptr {}", acc))?;
        let fc = self.label("qfc");
        let fb = self.label("qfb");
        let fe = self.label("qfe");
        let ft = self.label("qft");
        self.br(&fc)?;
        self.place(&fc);
        let jv = self.op(format!("load i64, ptr {}", j))?;
        let more = self.op(format!("icmp slt i64 {}, {}", jv, hi64))?;
        self.condbr(&more, &fb, &fe)?;
        self.place(&fb);
        let kv = self.op(format!("sext i64 {} to i128", jv))?;
        let b = self.cfast(&e.body, &e.var, &kv, &jv, None)?;
        let av = self.op(format!("load i1, ptr {}", acc))?;
        let na = self.op(format!("and i1 {}, {}", av, b))?;
        self.ins(format!("store i1 {}, ptr {}", na, acc))?;
        let j2 = self.op(format!("add i64 {}, 1", jv))?;
        self.ins(format!("store i64 {}, ptr {}", j2, j))?;
        self.br(&fc)?;
        self.place(&fe);
        let all = self.op(format!("load i1, ptr {}", acc))?;
        self.condbr(&all, &ft, &slow)?;
        self.place(&ft);
        let n64 = self.op(format!("trunc i128 {} to i64", n))?;
        self.spend_n(&n64)?;
        self.br(el)?;
        self.place(&slow);
        Ok(())
    }

    /// P3: a fast_forall body as straight-line IR (i1 and/or instead of short circuits, unchecked reads).
    /// P3-F1: xs (array name -> already loaded i64 SSA value) makes x[t] reuse that value instead of loading
    fn cfast(&mut self, e: &Expr, var: &str, kv: &str, jv: &str, xs: Option<&HashMap<String, String>>) -> R<String> {
        match e {
            Expr::BoolLit(b) => Ok(if b.value { "true" } else { "false" }.to_string()),
            Expr::IntLit(l) => Ok(l.value.parse::<i128>().map_err(|x| x.to_string())?.to_string()),
            Expr::Var(v) if v.name == var => Ok(kv.to_string()),
            Expr::Unary(u) => {
                let x = self.cfast(&u.operand, var, kv, jv, xs)?;
                self.op(format!("xor i1 {}, true", x))
            }
            Expr::Binary(b) if b.op == "&&" || b.op == "||" || b.op == "==>" => {
                let mut l = self.cfast(&b.left, var, kv, jv, xs)?;
                if b.op == "==>" {
                    l = self.op(format!("xor i1 {}, true", l))?;
                }
                let r = self.cfast(&b.right, var, kv, jv, xs)?;
                self.op(format!("{} i1 {}, {}", if b.op == "&&" { "and" } else { "or" }, l, r))
            }
            Expr::Binary(b) => {
                let cmp = match b.op.as_str() {
                    "==" => "eq",
                    "!=" => "ne",
                    "<" => "slt",
                    "<=" => "sle",
                    ">" => "sgt",
                    _ => "sge",
                };
                let l = self.cfast(&b.left, var, kv, jv, xs)?;
                let r = self.cfast(&b.right, var, kv, jv, xs)?;
                self.op(format!("icmp {} i128 {}, {}", cmp, l, r))
            }
            Expr::Index(ix) if xs.is_some() => {
                let name = match &*ix.array {
                    Expr::Var(v) => v.name.clone(),
                    _ => return Err("llvmgen: internal error: fast_forall array".to_string()),
                };
                let x = xs.and_then(|m| m.get(&name)).cloned().ok_or_else(|| format!("'{}'", name))?;
                self.op(format!("sext i64 {} to i128", x))
            }
            Expr::Index(ix) => {
                let (name, u8a) = match &*ix.array {
                    Expr::Var(v) => (v.name.clone(), v.ty.as_deref() == Some("array_u8")),
                    _ => return Err("llvmgen: internal error: fast_forall array".to_string()),
                };
                let base = self.cenv.get(&name).map(|v| v.a.clone()).ok_or_else(|| format!("'{}'", name))?;
                let i = match &*ix.index {
                    Expr::Binary(b) => match &*b.right {
                        Expr::IntLit(l) => {
                            let c = l.value.parse::<i128>().map_err(|x| x.to_string())?;
                            self.op(format!("{} i64 {}, {}", if b.op == "+" { "add" } else { "sub" }, jv, c))?
                        }
                        _ => return Err("llvmgen: internal error: fast_forall index".to_string()),
                    },
                    _ => jv.to_string(),
                };
                if u8a {
                    let g = self.op(format!("getelementptr inbounds i8, ptr {}, i64 {}", base, i))?;
                    let x = self.op(format!("load i8, ptr {}", g))?;
                    return self.op(format!("zext i8 {} to i128", x));
                }
                let g = self.op(format!("getelementptr inbounds i64, ptr {}, i64 {}", base, i))?;
                let x = self.op(format!("load i64, ptr {}", g))?;
                self.op(format!("sext i64 {} to i128", x))
            }
            _ => Ok(self.cev(e)?.a),
        }
    }

    pub fn cquant(&mut self, e: &Expr) -> R<V> {
        let x = match e {
            Expr::Quant(q) => q,
            _ => return Err("llvmgen: internal error: cquant on non-quantifier".to_string()),
        };
        if x.kind == "forall" {
            if let Some(sorted_arr) = crate::emit_common::match_sorted_requires(e) {
                // G2 B2: if the matched array is a str, skip the fast path and take the general path
                let is_str = self.cenv.get(&sorted_arr).map_or(false, |v| is_strv(&v.ty));
                if !is_str {
                    let q = self.nq;
                    self.nq += 1;
                    return self.csorted(&sorted_arr, q);
                }
            }
        }
        let lo = self.cev(&x.lo)?;
        let hi = self.cev(&x.hi)?;
        let q = self.nq;
        self.nq += 1;
        let k = format!("%q{}.k", q);
        let r = format!("%q{}.r", q);
        let h = format!("%q{}.h", q);
        let kind = x.kind.as_str();
        let is_mm = kind == "max" || kind == "min";
        self.allocas.push(format!("{} = alloca i128", k));
        let rty = if kind == "forall" || kind == "exists" { "i1" } else { "i128" };
        self.allocas.push(format!("{} = alloca {}", r, rty));
        if is_mm {
            self.allocas.push(format!("{} = alloca i1", h));
        }
        self.ins(format!("store i128 {}, ptr {}", lo.a, k))?;
        if kind == "forall" {
            self.ins(format!("store i1 true, ptr {}", r))?;
        } else if kind == "exists" {
            self.ins(format!("store i1 false, ptr {}", r))?;
        } else {
            self.ins(format!("store i128 0, ptr {}", r))?;
        }
        if is_mm {
            self.ins(format!("store i1 false, ptr {}", h))?;
        }
        let cl = self.label("qc");
        let bl = self.label("qb");
        let nl = self.label("qn");
        let el = self.label("qe");
        let reads = {
            let me = &*self;
            let ok = |x: &Expr| match x {
                Expr::Var(v) => me.cenv.get(&v.name).map_or(false, |c| {
                    c.k == Kind::Arr && (v.ty.as_deref() == Some("array_u8") || c.layout == "i64")
                }),
                _ => false,
            };
            crate::emit_common::fast_forall(x, &ok)
        };
        if let Some(reads) = reads {
            self.cfastpre(x, &reads, &lo.a, &hi.a, q, &el)?;
        }
        self.br(&cl)?;
        self.place(&cl);
        let kv = self.op(format!("load i128, ptr {}", k))?;
        let c = self.op(format!("icmp slt i128 {}, {}", kv, hi.a))?;
        self.condbr(&c, &bl, &el)?;
        self.place(&bl);
        self.spend()?;
        let saved = self.cenv.get(&x.var).cloned();
        self.cenv.insert(x.var.clone(), V::new(Kind::I128, kv.clone()));
        let body = &x.body;
        let b = self.cregion(&|g| g.cev(body))?;
        match saved {
            None => {
                self.cenv.remove(&x.var);
            }
            Some(s) => {
                self.cenv.insert(x.var.clone(), s);
            }
        }
        if kind == "forall" || kind == "exists" {
            let stop = self.label("qstop");
            if kind == "forall" {
                self.condbr(&b.a, &nl, &stop)?;
            } else {
                self.condbr(&b.a, &stop, &nl)?;
            }
            self.place(&stop);
            self.ins(format!("store i1 {}, ptr {}", if kind == "forall" { "false" } else { "true" }, r))?;
            self.br(&el)?;
        } else {
            let old = self.op(format!("load i128, ptr {}", r))?;
            let nv;
            if kind == "sum" {
                nv = self.checked("i128", "sadd", "+", &old, &b.a)?;
            } else if kind == "count" {
                let z = self.op(format!("zext i1 {} to i128", b.a))?;
                nv = self.op(format!("add i128 {}, {}", old, z))?;
            } else {
                let hv = self.op(format!("load i1, ptr {}", h))?;
                let cmp = self.op(format!("icmp {} i128 {}, {}", if kind == "max" { "sgt" } else { "slt" }, b.a, old))?;
                let nh = self.op(format!("xor i1 {}, true", hv))?;
                let better = self.op(format!("or i1 {}, {}", nh, cmp))?;
                nv = self.op(format!("select i1 {}, i128 {}, i128 {}", better, b.a, old))?;
                self.ins(format!("store i1 true, ptr {}", h))?;
            }
            self.ins(format!("store i128 {}, ptr {}", nv, r))?;
            self.br(&nl)?;
        }
        self.place(&nl);
        let k2 = self.op(format!("add i128 {}, 1", kv))?;
        self.ins(format!("store i128 {}, ptr {}", k2, k))?;
        self.br(&cl)?;
        self.place(&el);
        if is_mm {
            let hv = self.op(format!("load i1, ptr {}", h))?;
            self.guard(&hv, Fail::Panic1("pruv_panic_undefined", format!("{} over an empty range", kind)), false)?;
        }
        let res = self.op(format!("load {}, ptr {}", rty, r))?;
        Ok(V::new(if rty == "i1" { Kind::I1 } else { Kind::I128 }, res))
    }
}

fn cmp_name(op: &str) -> R<&'static str> {
    match op {
        "==" => Ok("eq"),
        "!=" => Ok("ne"),
        "<" => Ok("slt"),
        "<=" => Ok("sle"),
        ">" => Ok("sgt"),
        ">=" => Ok("sge"),
        _ => Err(format!("'{}'", op)),
    }
}

/// P3-F1: emit_common::fuse_map(f) when the result and the fused parameter arrays are plain i64 heap arrays, else None
fn fuse_plan_ll(m: &Mod, f: &Function) -> Option<crate::emit_common::FusePlan> {
    let plan = crate::emit_common::fuse_map(f)?;
    let l = match f.body.first() {
        Some(Stmt::Let(l)) => l,
        _ => return None,
    };
    if m.stack.contains(l) {
        return None;
    }
    if m.storage.layout_of(f) != "i64" || m.storage.layout_of(&l.value) != "i64" {
        return None;
    }
    if f.params.iter().any(|p| plan.arrs.contains(&p.name) && m.storage.layout_of(p) != "i64") {
        return None;
    }
    Some(plan)
}

/// P3-F1: the map element expression of a fused loop with wrapping i64 arithmetic (no nsw)
fn fuse_elem_ll(g: &mut FnGen, e: &Expr, xs: &HashMap<String, String>) -> R<String> {
    match e {
        Expr::IntLit(l) => Ok(l.value.clone()),
        Expr::Index(ix) => {
            let name = match &*ix.array {
                Expr::Var(v) => v.name.clone(),
                _ => return Err("llvmgen: internal error: fuse array".to_string()),
            };
            xs.get(&name).cloned().ok_or_else(|| format!("'{}'", name))
        }
        Expr::Binary(b) => {
            let l = fuse_elem_ll(g, &b.left, xs)?;
            let r = fuse_elem_ll(g, &b.right, xs)?;
            let op = match b.op.as_str() {
                "+" => "add",
                "-" => "sub",
                "*" => "mul",
                other => return Err(format!("'{}'", other)),
            };
            g.op(format!("{} i64 {}, {}", op, l, r))
        }
        other => Err(format!("llvmgen: internal error: fuse element {:?}", other)),
    }
}

/// P3-F1 (docs/P3_FUSE_DESIGN.md decision 5): the fused requires path of an element-wise map, emitted after the prefix
/// requires. A failed guard or a set flag continues at the slow path (the trailing requires) in block fzs.
fn fuse_ll(g: &mut FnGen, f: &Function, plan: &crate::emit_common::FusePlan) -> R<()> {
    let arr = |g: &FnGen, x: &str| -> R<V> { g.cenv.get(x).cloned().ok_or_else(|| format!("'{}'", x)) };
    let n = arr(g, &plan.n_arr)?.b;
    let mut conds: Vec<String> = Vec::new();
    for x in plan.arrs.iter().filter(|x| **x != plan.n_arr) {
        let xb = arr(g, x)?.b;
        conds.push(g.op(format!("icmp eq i64 {}, {}", xb, n))?);
    }
    let zx = g.op(format!("zext i64 {} to i128", n))?;
    let need = g.op(format!("mul i128 {}, {}", zx, plan.arrs.len()))?;
    let budget = match &g.budget {
        Some(b) => b.clone(),
        None => return Err("llvmgen: internal error: no iteration budget slot".to_string()),
    };
    let bl = g.op(format!("load i64, ptr {}", budget))?;
    let bw = g.op(format!("sext i64 {} to i128", bl))?;
    conds.push(g.op(format!("icmp sle i128 {}, {}", need, bw))?);
    let mut ok = conds[0].clone();
    for c in &conds[1..] {
        ok = g.op(format!("and i1 {}, {}", ok, c))?;
    }
    let pre = g.label("fzp");
    let slow = g.label("fzs");
    g.condbr(&ok, &pre, &slow)?;
    g.place(&pre);
    let p = g.op(format!("call ptr @pruv_alloc(i64 {}, i64 0)", n))?;
    let q = g.nq;
    g.nq += 1;
    let j = format!("%q{}.j", q);
    let bad = format!("%q{}.f", q);
    g.allocas.push(format!("{} = alloca i64", j));
    g.allocas.push(format!("{} = alloca i1", bad));
    g.ins(format!("store i64 0, ptr {}", j))?;
    g.ins(format!("store i1 false, ptr {}", bad))?;
    let fc = g.label("fzc");
    let fb = g.label("fzb");
    let fe = g.label("fze");
    let fr = g.label("fzr");
    let ff = g.label("fzf");
    g.br(&fc)?;
    g.place(&fc);
    let jv = g.op(format!("load i64, ptr {}", j))?;
    let lt = g.op(format!("icmp slt i64 {}, {}", jv, n))?;
    g.condbr(&lt, &fb, &fe)?;
    g.place(&fb);
    let mut xs: HashMap<String, String> = HashMap::new();
    for x in &plan.arrs {
        let xa = arr(g, x)?.a;
        let gp = g.op(format!("getelementptr inbounds i64, ptr {}, i64 {}", xa, jv))?;
        let v = g.op(format!("load i64, ptr {}", gp))?;
        xs.insert(x.clone(), v);
    }
    let kv = g.op(format!("sext i64 {} to i128", jv))?;
    let mut acc = g.op(format!("load i1, ptr {}", bad))?;
    for r in &f.requires[plan.start..] {
        let qn = match r {
            Expr::Quant(qn) => qn,
            _ => return Err("llvmgen: internal error: fuse requires".to_string()),
        };
        let pv = g.cfast(&qn.body, &qn.var, &kv, &jv, Some(&xs))?;
        let nv = g.op(format!("xor i1 {}, true", pv))?;
        acc = g.op(format!("or i1 {}, {}", acc, nv))?;
    }
    g.ins(format!("store i1 {}, ptr {}", acc, bad))?;
    let ev = fuse_elem_ll(g, &plan.elem, &xs)?;
    let gp = g.op(format!("getelementptr inbounds i64, ptr {}, i64 {}", p, jv))?;
    g.ins(format!("store i64 {}, ptr {}", ev, gp))?;
    let j1 = g.op(format!("add i64 {}, 1", jv))?;
    g.ins(format!("store i64 {}, ptr {}", j1, j))?;
    g.br(&fc)?;
    g.place(&fe);
    let bv = g.op(format!("load i1, ptr {}", bad))?;
    g.condbr(&bv, &ff, &fr)?;
    g.place(&fr);
    let a = g.op(format!("insertvalue {} undef, ptr {}, 0", ARR, p))?;
    let r1 = g.op(format!("insertvalue {} {}, i64 {}, 1", ARR, a, n))?;
    let r2 = g.op(format!("insertvalue {} {}, i64 {}, 2", ARR, r1, n))?; // P3: cap == len
    g.term(format!("ret {} {}", ARR, r2))?;
    g.place(&ff);
    g.ins(format!("call void @pruv_free(ptr {}, i64 {})", p, n))?;
    g.br(&slow)?;
    g.place(&slow);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn entry_of(
    m: &mut Mod,
    tt: &TypeTable,
    funcs: &HashMap<String, &Function>,
    f: &Function,
    trust: bool,
    budget: u64,
    cost: bool,
    verified: bool,
) -> R<String> {
    let mut g = FnGen::new(m, tt, funcs, f, true);
    let mut params = ir_params(f, tt, &g.m.resize_params);
    let rt = ir_ret(tt, f); // G9 A3: "void" for an extended result (output storage is the leading `ptr %out`)
    if trust && !cost {
        g.place("start");
    } else {
        g.bind_contract_params()?;
        let bname = "%req.budget".to_string();
        g.budget = Some(bname.clone());
        g.allocas.push(format!("{} = alloca i64", bname));
        g.ins(format!("store i64 {}, ptr {}", budget, bname))?;
        // P3-F1: verified checked entries
        let plan = if verified && !trust && !cost { fuse_plan_ll(&*g.m, f) } else { None };
        for (k, r) in f.requires.iter().enumerate().filter(|_| !trust) {
            if let Some(p) = &plan {
                if k == p.start {
                    fuse_ll(&mut g, f, p)?;
                }
            }
            let c = g.cregion(&|g2| g2.cev(r))?;
            // G4: the source-qualified identity, not the stem
            let disp = g.m.fn_disp.get(&f.name).cloned().unwrap_or_else(|| f.name.clone());
            let msg_fn = g.m.string(&disp);
            let msg_ex = g.m.string(&crate::emit_common::show(r));
            g.guard(&c.a, Fail::Requires(msg_fn, msg_ex), false)?;
        }
    }
    if cost {
        let expr = f.cost.as_ref().ok_or_else(|| "missing cost bound".to_string())?;
        // LG15 (G-LG12-5): a G9 diagnostic raised here names the cost clause
        let value = g.cregion(&|g2| g2.cev(expr)).map_err(|e| match e.strip_prefix(G9_DIAG_TAG) {
            Some(rest) => format!("{}{}", G9_COST_DIAG_TAG, rest),
            None => e,
        })?;
        g.term(format!("ret i128 {}", value.a))?;
        params.retain(|(_, n)| n != "%out");
        return Ok(g.render(&format!("define internal i128 @{}__pruv_cost({})", mangle(&f.name), sig_text(&params, Some(f)))));
    }
    let args = params.iter().map(|(t, n)| format!("{} {}", t, n)).collect::<Vec<_>>().join(", ");
    let callee = format!("@{}__pruv_body", mangle(&f.name));
    if f.ret == "unit" || rt == "void" {
        g.ins(format!("call void {}({})", callee, args))?;
        g.term("ret void".to_string())?;
    } else {
        let r = g.op(format!("call {} {}({})", rt, callee, args))?;
        g.term(format!("ret {} {}", rt, r))?;
    }
    Ok(g.render(&format!("define internal {} @{}__pruv_entry({})", rt, mangle(&f.name), sig_text(&params, Some(f)))))
}

// ======================================================================================================================
// G9 A3: recursive values and aggregate buffer fields (docs/G9_SPEC.md 4, 6, 7, 13); counterpart of the g9_* section of
// llvmgen.py (see its design note: layout, values, ownership, calls and the stable helper names).
// ======================================================================================================================

/// [(variant name or None, tag or None, fields)] in declaration order
type Var9<'t> = (Option<&'t str>, Option<usize>, &'t [FieldDecl]);

/// A function that stands in for `None` (the helper generators have no function): never read by the helper code.
fn dummy_fn() -> Function {
    Function { borrow_from: None, borrowed_result: false,
        line: 0,
        col: 0,
        name: String::new(),
        params: vec![],
        ret: "unit".to_string(),
        effects: vec![],
        requires: vec![],
        ensures: vec![],
        body: vec![],
        ret_refine: None,
        tparams: vec![],
        foreign: None,
        cost: None, decreases: None, small_bound: None, x_auto: None, opaque: false, pred: false, attribute: None,
    }
}

fn g9_gep(g: &mut FnGen, t: &str, p: &str, idx: usize) -> R<String> {
    g.op(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 {}", t, p, idx))
}

fn g9_variants(d: &TypeDecl) -> Vec<Var9<'_>> {
    match d {
        TypeDecl::Struct(s) => vec![(None, None, s.fields.as_slice())],
        TypeDecl::Enum(e) => e
            .variants
            .iter()
            .enumerate()
            .map(|(i, v)| (Some(v.name.as_str()), Some(i), v.fields.as_slice()))
            .collect(),
    }
}

fn g9_field_gep(g: &mut FnGen, ty: &str, p: &str, vn: Option<&str>, i: usize) -> R<String> {
    let t = variant_type(g.tt, ty, vn, false);
    let idx = field_index(g.tt, ty, vn, i, false)?;
    g9_gep(g, &t, p, idx)
}

/// emit(variant name, fields) for the variants in vs. A struct runs it inline. An enum switches on the tag of the value
/// at `base` and continues, on every path, at a join label that is left as the current block.
fn g9_each_variant<'a>(
    g: &mut FnGen<'a>,
    ty: &str,
    d: &TypeDecl,
    base: &str,
    vs: &[Var9<'_>],
    emit: &mut dyn FnMut(&mut FnGen<'a>, Option<&str>, &[FieldDecl]) -> R<()>,
) -> R<()> {
    if let TypeDecl::Struct(_) = d {
        for (vn, _t, fs) in vs {
            emit(g, *vn, fs)?;
        }
        return Ok(());
    }
    if vs.is_empty() {
        return Ok(());
    }
    let tp = g.tag_ptr(ty, base)?;
    let tag = g.op(format!("load i64, ptr {}", tp))?;
    let done = g.label("gdone");
    let mut lbls: Vec<String> = Vec::new();
    for _ in vs {
        lbls.push(g.label("gv"));
    }
    let mut cases: Vec<String> = Vec::new();
    for ((_vn, t, _fs), l) in vs.iter().zip(&lbls) {
        cases.push(format!("i64 {}, label %{}", t.unwrap_or(0), l));
    }
    g.term(format!("switch i64 {}, label %{} [ {} ]", tag, done, cases.join(" ")))?;
    for ((vn, _t, fs), l) in vs.iter().zip(&lbls) {
        g.place(l);
        emit(g, *vn, fs)?;
        g.br(&done)?;
    }
    g.place(&done);
    Ok(())
}

/// (size, align) SSA names of the node type T computed by the backend from the target layout
fn g9_node_layout(g: &mut FnGen, t: &str) -> R<(String, String)> {
    let z = g.op(format!("getelementptr {}, ptr null, i32 1", t))?;
    let size = g.op(format!("ptrtoint ptr {} to i64", z))?;
    let a = g.op(format!("getelementptr {{ i8, {} }}, ptr null, i32 0, i32 1", t))?;
    let align = g.op(format!("ptrtoint ptr {} to i64", a))?;
    Ok((size, align))
}

fn g9_drop_text(m: &mut Mod, tt: &TypeTable, d: &TypeDecl) -> R<String> {
    let ty = d.name().to_string();
    let df = dummy_fn();
    let funcs: HashMap<String, &Function> = HashMap::new();
    let mut g = FnGen::new(&mut *m, tt, &funcs, &df, true);
    g.place("start");
    let vs: Vec<Var9<'_>> = g9_variants(d)
        .into_iter()
        .filter(|(_, _, fs)| fs.iter().any(|f| is_arr_name(&f.ty) || is_collection(&f.ty) || g.is_ext(&f.ty)))
        .collect();
    g9_each_variant(&mut g, &ty, d, "%v", &vs, &mut |g, vn, fs| {
        for (i, f) in fs.iter().enumerate() {
            let ft = f.ty.as_str();
            if !(is_arr_name(ft) || is_collection(ft) || g.is_ext(ft)) {
                continue;
            }
            let gp = g9_field_gep(g, &ty, "%v", vn, i)?;
            if g.fb(&ty, vn, &f.name) {
                // a boxed child: its contents first, then its node
                let c = g.op(format!("load ptr, ptr {}", gp))?;
                g.ins(format!("call void @pruv_dropnode__{}(ptr {})", ft, c))?;
            } else if is_arr_name(ft) {
                let x = g.op(format!("load {}, ptr {}", ARR, gp))?;
                let av = g.unpack(&x, ft, false)?; // P3: freed with its cap
                g.free_val(&av)?;
            } else if is_collection(ft) {
                g.drop_slot(&V { ty: Some(ft.to_string()), ..V::new(Kind::Coll, gp) })?;
            } else {
                g.drop_agg_slot(ft, &gp)?;
            }
        }
        Ok(())
    })?;
    g.term("ret void".to_string())?;
    Ok(g.render(&format!("define internal void @pruv_drop__{}(ptr %v)", d.name())))
}

fn g9_clone_text(m: &mut Mod, tt: &TypeTable, d: &TypeDecl) -> R<String> {
    let ty = d.name().to_string();
    let tl = lty(&ty);
    let df = dummy_fn();
    let funcs: HashMap<String, &Function> = HashMap::new();
    let mut g = FnGen::new(&mut *m, tt, &funcs, &df, true);
    g.place("start");
    // shallow copy: scalars, inactive null/zero slots; owners are fixed below
    let v0 = g.op(format!("load {}, ptr %src", tl))?;
    g.ins(format!("store {} {}, ptr %dst", tl, v0))?;
    let vs: Vec<Var9<'_>> = g9_variants(d)
        .into_iter()
        .filter(|(_, _, fs)| fs.iter().any(|f| is_arr_name(&f.ty) || is_collection(&f.ty) || g.is_ext(&f.ty)))
        .collect();
    g9_each_variant(&mut g, &ty, d, "%src", &vs, &mut |g, vn, fs| {
        for (i, f) in fs.iter().enumerate() {
            let ft = f.ty.as_str();
            if !(is_arr_name(ft) || is_collection(ft) || g.is_ext(ft)) {
                continue;
            }
            let gs = g9_field_gep(g, &ty, "%src", vn, i)?;
            let gd = g9_field_gep(g, &ty, "%dst", vn, i)?;
            if g.fb(&ty, vn, &f.name) {
                let c = g.op(format!("load ptr, ptr {}", gs))?;
                let nb = g.op(format!("call ptr @pruv_clonenode__{}(ptr {})", ft, c))?;
                g.ins(format!("store ptr {}, ptr {}", nb, gd))?;
            } else if is_arr_name(ft) {
                let x = g.op(format!("load {}, ptr {}", ARR, gs))?;
                let p = g.op(format!("extractvalue {} {}, 0", ARR, x))?;
                let n = g.op(format!("extractvalue {} {}, 1", ARR, x))?;
                let np = if is_bytes_ty(&elem_ty(ft)) {
                    g.op(format!("call ptr @pruv_copy_v2(ptr {}, i64 {}, i64 1, i64 1)", p, n))?
                } else {
                    g.op(format!("call ptr @pruv_copy(ptr {}, i64 {})", p, n))?
                };
                let a1 = g.op(format!("insertvalue {} {}, ptr {}, 0", ARR, x, np))?;
                let a2 = g.op(format!("insertvalue {} {}, i64 {}, 2", ARR, a1, n))?; // P3: the new block holds exactly len elements
                g.ins(format!("store {} {}, ptr {}", ARR, a2, gd))?;
            } else if let Some(elem) = native_vec_elem(ft) {
                g.m.vec_elems.insert(elem.to_string());
                g.ins(format!("call void @pruv_vclone__{elem}(ptr {gs}, ptr {gd})"))?;
            } else if is_collection(ft) {
                let handle = g.op(format!("load ptr, ptr {gs}"))?;
                let nn = g.op(format!("icmp ne ptr {handle}, null"))?;
                let clone = g.label("gclone");
                let done = g.label("gdone");
                g.condbr(&nn, &clone, &done)?;
                g.place(&clone);
                let sym = crate::foreign_shim::clone_sym(ft)?;
                let value = g.g6_call(&sym, "ptr", &[("ptr".to_string(), handle)])?;
                g.ins(format!("store ptr {value}, ptr {gd}"))?;
                g.br(&done)?;
                g.place(&done);
            } else {
                g.ins(format!("call void @pruv_clone__{}(ptr {}, ptr {})", ft, gs, gd))?;
            }
        }
        Ok(())
    })?;
    g.term("ret void".to_string())?;
    Ok(g.render(&format!("define internal void @pruv_clone__{}(ptr %src, ptr %dst)", d.name())))
}

/// box / dropnode / clonenode of a type that is the target of a boxed edge
fn g9_node_texts(m: &mut Mod, tt: &TypeTable, d: &TypeDecl) -> R<Vec<String>> {
    let n = d.name().to_string();
    let tl = lty(&n);
    let mut out: Vec<String> = Vec::new();
    let df = dummy_fn();
    let funcs: HashMap<String, &Function> = HashMap::new();
    {
        let mut g = FnGen::new(&mut *m, tt, &funcs, &df, true);
        g.place("start");
        let (size, align) = g9_node_layout(&mut g, &tl)?;
        let p = g.op(format!("call ptr @pruv_node_alloc_v1(i64 {}, i64 {})", size, align))?;
        g.ins(format!("store {} %v, ptr {}", tl, p))?;
        g.term(format!("ret ptr {}", p))?;
        out.push(g.render(&format!("define internal ptr @pruv_box__{}({} %v)", n, tl)));
    }
    {
        let mut g = FnGen::new(&mut *m, tt, &funcs, &df, true);
        g.place("start");
        let end = if g.m.owned_collection_fields {
            let live = g.op("icmp ne ptr %n, null".to_string())?;
            let drop = g.label("ndrop");
            let end = g.label("ndone");
            g.condbr(&live, &drop, &end)?;
            g.place(&drop);
            Some(end)
        } else {
            None
        };
        g.ins(format!("call void @pruv_drop__{}(ptr %n)", n))?;
        let (size, align) = g9_node_layout(&mut g, &tl)?;
        g.ins(format!("call void @pruv_node_free_v1(ptr %n, i64 {}, i64 {})", size, align))?;
        if let Some(end) = end {
            g.br(&end)?;
            g.place(&end);
        }
        g.term("ret void".to_string())?;
        out.push(g.render(&format!("define internal void @pruv_dropnode__{}(ptr %n)", n)));
    }
    {
        let mut g = FnGen::new(&mut *m, tt, &funcs, &df, true);
        g.place("start");
        let (size, align) = g9_node_layout(&mut g, &tl)?;
        let p = g.op(format!("call ptr @pruv_node_alloc_v1(i64 {}, i64 {})", size, align))?;
        g.ins(format!("call void @pruv_clone__{}(ptr %src, ptr {})", n, p))?;
        g.term(format!("ret ptr {}", p))?;
        out.push(g.render(&format!("define internal ptr @pruv_clonenode__{}(ptr %src)", n)));
    }
    Ok(out)
}

/// size / equality walk into t: a nominal value, or Option / Result of one
/// Visit the initialized elements in index order. The callback may emit its own blocks.
fn g9_vec_each(
    g: &mut FnGen,
    len: &str,
    visit: &mut impl FnMut(&mut FnGen, &str) -> R<()>,
) -> R<()> {
    let start = g.cur.clone();
    let head = g.label("vloop");
    let body = g.label("vbody");
    let latch = g.label("vnext");
    let done = g.label("vend");
    g.br(&head)?;
    g.place(&head);
    let next = g.tmp();
    let i = g.op(format!("phi i64 [ 0, %{start} ], [ {next}, %{latch} ]"))?;
    let more = g.op(format!("icmp ult i64 {i}, {len}"))?;
    g.condbr(&more, &body, &done)?;
    g.place(&body);
    visit(g, &i)?;
    g.br(&latch)?;
    g.place(&latch);
    g.ins(format!("{next} = add i64 {i}, 1"))?;
    g.br(&head)?;
    g.place(&done);
    Ok(())
}

fn g9_vec_parts(g: &mut FnGen, p: &str) -> R<(String, String, String)> {
    let v = g.op(format!("load {ARR}, ptr {p}"))?;
    let data = g.op(format!("extractvalue {ARR} {v}, 0"))?;
    let len = g.op(format!("extractvalue {ARR} {v}, 1"))?;
    let cap = g.op(format!("extractvalue {ARR} {v}, 2"))?;
    Ok((data, len, cap))
}

fn g9_vec_texts(m: &mut Mod, tt: &TypeTable, elem: &str, budget: Option<u64>) -> R<Vec<String>> {
    let tl = lty(elem);
    let extended = is_ext_t(tt, elem);
    let df = dummy_fn();
    let funcs = HashMap::new();
    let mut out = Vec::new();
    {
        let mut g = FnGen::new(m, tt, &funcs, &df, true);
        g.place("start");
        let (data, len, cap) = g9_vec_parts(&mut g, "%v")?;
        if extended {
            g9_vec_each(&mut g, &len, &mut |g, i| {
                let p = g.op(format!("getelementptr inbounds {tl}, ptr {data}, i64 {i}"))?;
                g.ins(format!("call void @pruv_drop__{elem}(ptr {p})"))
            })?;
        }
        let free = g.label("vfree");
        let done = g.label("vdone");
        let nonempty = g.op(format!("icmp ne i64 {cap}, 0"))?;
        g.condbr(&nonempty, &free, &done)?;
        g.place(&free);
        let (size, align) = g9_node_layout(&mut g, &tl)?;
        let bytes = g.op(format!("mul i64 {cap}, {size}"))?;
        g.ins(format!("call void @pruv_node_free_v1(ptr {data}, i64 {bytes}, i64 {align})"))?;
        g.br(&done)?;
        g.place(&done);
        g.term("ret void".to_string())?;
        out.push(g.render(&format!("define internal void @pruv_vdrop__{elem}(ptr %v)")));
    }
    {
        let mut g = FnGen::new(m, tt, &funcs, &df, true);
        g.place("start");
        let (src, len, _) = g9_vec_parts(&mut g, "%src")?;
        let empty = g.label("vempty");
        let copy = g.label("vcopy");
        let zero = g.op(format!("icmp eq i64 {len}, 0"))?;
        g.condbr(&zero, &empty, &copy)?;
        g.place(&empty);
        g.ins(format!("store {ARR} zeroinitializer, ptr %dst"))?;
        g.term("ret void".to_string())?;
        g.place(&copy);
        let (size, align) = g9_node_layout(&mut g, &tl)?;
        let bytes = g.op(format!("mul i64 {len}, {size}"))?;
        let dst = g.op(format!("call ptr @pruv_node_realloc_v1(ptr null, i64 0, i64 {bytes}, i64 {align})"))?;
        if extended {
            g9_vec_each(&mut g, &len, &mut |g, i| {
                let sp = g.op(format!("getelementptr inbounds {tl}, ptr {src}, i64 {i}"))?;
                let dp = g.op(format!("getelementptr inbounds {tl}, ptr {dst}, i64 {i}"))?;
                g.ins(format!("call void @pruv_clone__{elem}(ptr {sp}, ptr {dp})"))
            })?;
        } else {
            g.ins(format!("call void @llvm.memcpy.p0.p0.i64(ptr {dst}, ptr {src}, i64 {bytes}, i1 false)"))?;
        }
        let a = g.op(format!("insertvalue {ARR} zeroinitializer, ptr {dst}, 0"))?;
        let b = g.op(format!("insertvalue {ARR} {a}, i64 {len}, 1"))?;
        let c = g.op(format!("insertvalue {ARR} {b}, i64 {len}, 2"))?;
        g.ins(format!("store {ARR} {c}, ptr %dst"))?;
        g.term("ret void".to_string())?;
        out.push(g.render(&format!("define internal void @pruv_vclone__{elem}(ptr %src, ptr %dst)")));
    }
    {
        let mut g = FnGen::new(m, tt, &funcs, &df, true);
        g.place("start");
        let (data, len, cap) = g9_vec_parts(&mut g, "%v")?;
        let fail = |panic, drop_element| Fail::VecPush { elem: elem.to_string(), drop_element, panic };
        const BUDGET_PANIC: &str = "call void @pruv_panic_vec_push_v1(i64 1)";
        if g.m.foreign_requires {
            let bounded = g.op(format!("icmp ult i64 {len}, {}", 1u64 << 40))?;
            g.guard(&bounded, fail("call void @pruv_panic_vec_push_v1(i64 0)", extended), false)?;
        }
        let remaining = if let Some(budget) = budget {
            // Charge the snapshot before push, like the G6 shim.
            let enough = g.op(format!("icmp uge i64 {budget}, {len}"))?;
            g.guard(&enough, fail(BUDGET_PANIC, extended), false)?;
            Some(g.op(format!("sub i64 {budget}, {len}"))?)
        } else {
            None
        };
        let ready = g.cur.clone();
        let grow = g.label("vgrow");
        let push = g.label("vpush");
        let full = g.op(format!("icmp eq i64 {len}, {cap}"))?;
        g.condbr(&full, &grow, &push)?;
        g.place(&grow);
        let empty = g.op(format!("icmp eq i64 {cap}, 0"))?;
        let double = g.op(format!("mul i64 {cap}, 2"))?;
        let ncap = g.op(format!("select i1 {empty}, i64 4, i64 {double}"))?;
        let (size, align) = g9_node_layout(&mut g, &tl)?;
        let prod = g.op(format!("call {{ i64, i1 }} @llvm.umul.with.overflow.i64(i64 {ncap}, i64 {size})"))?;
        let bytes = g.op(format!("extractvalue {{ i64, i1 }} {prod}, 0"))?;
        let overflow = g.op(format!("extractvalue {{ i64, i1 }} {prod}, 1"))?;
        g.guard(&overflow, fail("call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)", extended), true)?;
        let grown = g.cur.clone();
        let old = g.op(format!("mul i64 {cap}, {size}"))?;
        let dst = g.op(format!("call ptr @pruv_node_realloc_v1(ptr {data}, i64 {old}, i64 {bytes}, i64 {align})"))?;
        g.ins(format!("store ptr {dst}, ptr %v"))?;
        let cp = g9_gep(&mut g, ARR, "%v", 2)?;
        g.ins(format!("store i64 {ncap}, ptr {cp}"))?;
        g.br(&push)?;
        g.place(&push);
        let dp = g.op(format!("phi ptr [ {data}, %{ready} ], [ {dst}, %{grown} ]"))?;
        let slot = g.op(format!("getelementptr inbounds {tl}, ptr {dp}, i64 {len}"))?;
        let value = g.op(format!("load {tl}, ptr %x"))?;
        g.ins(format!("store {tl} {value}, ptr {slot}"))?;
        let next = g.op(format!("add i64 {len}, 1"))?;
        let lp = g9_gep(&mut g, ARR, "%v", 1)?;
        g.ins(format!("store i64 {next}, ptr {lp}"))?;
        if let Some(remaining) = remaining {
            // The shim charges the prefix even for extended elements, then charges the new element.
            // The vector now owns the inserted element, so its drop handles both arguments.
            let enough = g.op(format!("icmp uge i64 {remaining}, {len}"))?;
            g.guard(&enough, fail(BUDGET_PANIC, false), false)?;
            let remaining = g.op(format!("sub i64 {remaining}, {len}"))?;
            let enough = g.op(format!("icmp uge i64 {remaining}, 1"))?;
            g.guard(&enough, fail(BUDGET_PANIC, false), false)?;
        }
        g.term("ret void".to_string())?;
        out.push(g.render(&format!("define internal void @pruv_vpush__{elem}(ptr %v, ptr %x)")));
    }
    Ok(out)
}

fn g9_walks(t: &str) -> bool {
    if native_vec_elem(t).is_some() {
        return true;
    }
    if is_gopt(t) {
        return g9_walks(g_arg(t)) || (t.starts_with("result<") && g9_walks(g_error(t)));
    }
    crate::emit_common::is_user_key(t)
}

/// i128 SSA value of weight(value stored at p) (G9_SPEC 3), or None when it is statically 0. Some / Ok weigh their
/// payload, None / Err 0; a nominal value weighs its size (one budget unit per node, inside pruv_csize). bud is the
/// budget pointer the size helper charges.
fn g9_weight(g: &mut FnGen, t: &str, p: &str, bud: &str) -> R<Option<String>> {
    if let Some(elem) = native_vec_elem(t) {
        g.m.need_size.insert(elem.to_string());
        let (data, len, _) = g9_vec_parts(g, p)?;
        let acc = g.g6_out("i128");
        g.ins(format!("store i128 0, ptr {acc}"))?;
        let tl = lty(elem);
        g9_vec_each(g, &len, &mut |g, i| {
            let p = g.op(format!("getelementptr inbounds {tl}, ptr {data}, i64 {i}"))?;
            let w = g.op(format!("call i128 @pruv_csize__{elem}(ptr {p}, ptr {bud})"))?;
            let prev = g.op(format!("load i128, ptr {acc}"))?;
            let next = g.checked("i128", "sadd", "+", &prev, &w)?;
            g.ins(format!("store i128 {next}, ptr {acc}"))
        })?;
        return Ok(Some(g.op(format!("load i128, ptr {acc}"))?));
    }
    if t.starts_with("result<") && g9_walks(g_error(t)) {
        let tl = st(t, false);
        let g0 = g9_gep(g, &tl, p, 0)?;
        let tag = g.op(format!("load i1, ptr {}", g0))?;
        let yes = g.label("gwy");
        let no = g.label("gwn");
        let end = g.label("gwe");
        g.condbr(&tag, &yes, &no)?;
        g.place(&yes);
        let px = g9_gep(g, &tl, p, 1)?;
        let w = g9_weight(g, g_arg(t), &px, bud)?.unwrap_or_else(|| "0".to_string());
        let wcur = g.cur.clone();
        g.br(&end)?;
        g.place(&no);
        let pe = g9_gep(g, &tl, p, 2)?;
        let we = g9_weight(g, g_error(t), &pe, bud)?.unwrap_or_else(|| "0".to_string());
        let ecur = g.cur.clone();
        g.br(&end)?;
        g.place(&end);
        return g.op(format!("phi i128 [ {}, %{} ], [ {}, %{} ]", w, wcur, we, ecur)).map(Some);
    }
    if is_gopt(t) {
        let x = g_arg(t);
        if !g9_walks(x) {
            return Ok(None);
        }
        let tl = st(t, false);
        let g0 = g9_gep(g, &tl, p, 0)?;
        let tag = g.op(format!("load i1, ptr {}", g0))?;
        let pl = g9_gep(g, &tl, p, 1)?;
        let lcur = g.cur.clone();
        let yes = g.label("gwy");
        let end = g.label("gwe");
        g.condbr(&tag, &yes, &end)?;
        g.place(&yes);
        let w = g9_weight(g, x, &pl, bud)?.unwrap_or_else(|| "None".to_string());
        let wcur = g.cur.clone();
        g.br(&end)?;
        g.place(&end);
        let r = g.op(format!("phi i128 [ 0, %{} ], [ {}, %{} ]", lcur, w, wcur))?;
        return Ok(Some(r));
    }
    if crate::emit_common::is_user_key(t) {
        let r = g.op(format!("call i128 @pruv_csize__{}(ptr {}, ptr {})", t, p, bud))?;
        return Ok(Some(r));
    }
    Ok(None)
}

/// 1 + the weights of the ACTIVE fields of one variant, checked i128 additions in field order
fn g9_size_fold(g: &mut FnGen, ty: &str, vn: Option<&str>, fs: &[FieldDecl]) -> R<String> {
    let mut acc = "1".to_string();
    for (i, f) in fs.iter().enumerate() {
        if !g9_walks(&f.ty) {
            continue;
        }
        let mut gp = g9_field_gep(g, ty, "%v", vn, i)?;
        if g.fb(ty, vn, &f.name) {
            gp = g.op(format!("load ptr, ptr {}", gp))?;
        }
        let w = g9_weight(g, &f.ty, &gp, "%bud")?.unwrap_or_else(|| "None".to_string());
        acc = g.checked("i128", "sadd", "+", &acc, &w)?;
    }
    Ok(acc)
}

/// pruv_csize__T: 1 + the weights of the ACTIVE fields, checked i128 additions in field order, one budget unit
fn g9_size_text(m: &mut Mod, tt: &TypeTable, d: &TypeDecl) -> R<String> {
    let ty = d.name().to_string();
    let df = dummy_fn();
    let funcs: HashMap<String, &Function> = HashMap::new();
    let mut g = FnGen::new(&mut *m, tt, &funcs, &df, true);
    g.place("start");
    g.budget = Some("%bud".to_string());
    g.spend()?;
    match d {
        TypeDecl::Struct(s) => {
            let acc = g9_size_fold(&mut g, &ty, None, &s.fields)?;
            g.term(format!("ret i128 {}", acc))?;
        }
        TypeDecl::Enum(_) => {
            g.allocas.push("%res = alloca i128".to_string());
            g.ins("store i128 1, ptr %res".to_string())?;
            let vs: Vec<Var9<'_>> = g9_variants(d)
                .into_iter()
                .filter(|(_, _, fs)| fs.iter().any(|f| g9_walks(&f.ty)))
                .collect();
            g9_each_variant(&mut g, &ty, d, "%v", &vs, &mut |g, vn, fs| {
                let acc = g9_size_fold(g, &ty, vn, fs)?;
                g.ins(format!("store i128 {}, ptr %res", acc))
            })?;
            let r = g.op("load i128, ptr %res".to_string())?;
            g.term(format!("ret i128 {}", r))?;
        }
    }
    Ok(g.render(&format!("define internal i128 @pruv_csize__{}(ptr %v, ptr %bud)", d.name())))
}

/// i1 SSA value: structural equality of the values stored at pa and pb (type t)
fn g9_eq_val(g: &mut FnGen, t: &str, pa: &str, pb: &str) -> R<String> {
    if is_gopt(t) {
        // Option<X> / Result<X>: equal tags, then the active payload only
        let tl = st(t, false);
        let x = g_arg(t);
        let ga = g9_gep(g, &tl, pa, 0)?;
        let ta = g.op(format!("load i1, ptr {}", ga))?;
        let gb = g9_gep(g, &tl, pb, 0)?;
        let tb = g.op(format!("load i1, ptr {}", gb))?;
        let te = g.op(format!("icmp eq i1 {}, {}", ta, tb))?;
        let lcur = g.cur.clone();
        let l1 = g.label("gqa");
        let lend = g.label("gqe");
        g.condbr(&te, &l1, &lend)?;
        g.place(&l1);
        let l2 = g.label("gqb");
        if t.starts_with("option<") {
            g.condbr(&ta, &l2, &lend)?; // None == None
            g.place(&l2);
            let ax = g9_gep(g, &tl, pa, 1)?;
            let bx = g9_gep(g, &tl, pb, 1)?;
            let e = g9_eq_val(g, x, &ax, &bx)?;
            let l2cur = g.cur.clone();
            g.br(&lend)?;
            g.place(&lend);
            return g.op(format!("phi i1 [ false, %{} ], [ true, %{} ], [ {}, %{} ]", lcur, l1, e, l2cur));
        }
        let l3 = g.label("gqc");
        g.condbr(&ta, &l2, &l3)?;
        g.place(&l2);
        let ax = g9_gep(g, &tl, pa, 1)?;
        let bx = g9_gep(g, &tl, pb, 1)?;
        let e = g9_eq_val(g, x, &ax, &bx)?;
        let l2cur = g.cur.clone();
        g.br(&lend)?;
        g.place(&l3);
        let ga2 = g9_gep(g, &tl, pa, 2)?;
        let et = g_error(t);
        let e3 = if et == "i64" {
            let ea = g.op(format!("load i64, ptr {}", ga2))?;
            let gb2 = g9_gep(g, &tl, pb, 2)?;
            let eb = g.op(format!("load i64, ptr {}", gb2))?;
            g.op(format!("icmp eq i64 {}, {}", ea, eb))?
        } else {
            let gb2 = g9_gep(g, &tl, pb, 2)?;
            g9_eq_val(g, et, &ga2, &gb2)?
        };
        let l3cur = g.cur.clone();
        g.br(&lend)?;
        g.place(&lend);
        return g.op(format!("phi i1 [ false, %{} ], [ {}, %{} ], [ {}, %{} ]", lcur, e, l2cur, e3, l3cur));
    }
    if t == "option" || t == "result" {
        // legacy Option<i64> / Result<i64>: { i1, i64 }
        let la = g.op(format!("load {}, ptr {}", OPT, pa))?;
        let lb = g.op(format!("load {}, ptr {}", OPT, pb))?;
        let ta = g.op(format!("extractvalue {} {}, 0", OPT, la))?;
        let tb = g.op(format!("extractvalue {} {}, 0", OPT, lb))?;
        let va = g.op(format!("extractvalue {} {}, 1", OPT, la))?;
        let vb = g.op(format!("extractvalue {} {}, 1", OPT, lb))?;
        let te = g.op(format!("icmp eq i1 {}, {}", ta, tb))?;
        let mut pe = g.op(format!("icmp eq i64 {}, {}", va, vb))?;
        if t == "option" {
            let nt = g.op(format!("xor i1 {}, true", ta))?;
            pe = g.op(format!("or i1 {}, {}", nt, pe))?;
        }
        return g.op(format!("and i1 {}, {}", te, pe));
    }
    if t == "i64" || t == "bool" {
        let ty = if t == "i64" { "i64" } else { "i1" };
        let la = g.op(format!("load {}, ptr {}", ty, pa))?;
        let lb = g.op(format!("load {}, ptr {}", ty, pb))?;
        return g.op(format!("icmp eq {} {}, {}", ty, la, lb));
    }
    if crate::emit_common::is_user_key(t) {
        return g.op(format!("call i1 @pruv_ceq__{}(ptr {}, ptr {}, ptr %bud)", t, pa, pb));
    }
    Err(format!("llvmgen: internal error: no structural equality for type {}", t))
}

/// the equal-fields chain of one variant: every field of fs must be equal, else jump to `fail`; ends with `ret i1 true`
fn g9_eq_conj(g: &mut FnGen, ty: &str, fail: &str, vn: Option<&str>, fs: &[FieldDecl]) -> R<()> {
    for (i, f) in fs.iter().enumerate() {
        let mut pa = g9_field_gep(g, ty, "%a", vn, i)?;
        let mut pb = g9_field_gep(g, ty, "%b", vn, i)?;
        if g.fb(ty, vn, &f.name) {
            pa = g.op(format!("load ptr, ptr {}", pa))?;
            pb = g.op(format!("load ptr, ptr {}", pb))?;
        }
        let e = g9_eq_val(g, &f.ty, &pa, &pb)?;
        let nxt = g.label("ceqn");
        g.condbr(&e, &nxt, fail)?;
        g.place(&nxt);
    }
    g.term("ret i1 true".to_string())
}

/// pruv_ceq__T: one budget unit, then equal tags and the fields of the active variant, left to right
fn g9_eq_text(m: &mut Mod, tt: &TypeTable, d: &TypeDecl) -> R<String> {
    let ty = d.name().to_string();
    let df = dummy_fn();
    let funcs: HashMap<String, &Function> = HashMap::new();
    let mut g = FnGen::new(&mut *m, tt, &funcs, &df, true);
    g.place("start");
    g.budget = Some("%bud".to_string());
    g.spend()?;
    let fail = g.label("ceqf");
    match d {
        TypeDecl::Struct(s) => g9_eq_conj(&mut g, &ty, &fail, None, &s.fields)?,
        TypeDecl::Enum(_) => {
            let pa = g.tag_ptr(&ty, "%a")?;
            let tl_ = g.op(format!("load i64, ptr {}", pa))?;
            let pb = g.tag_ptr(&ty, "%b")?;
            let tr = g.op(format!("load i64, ptr {}", pb))?;
            let same = g.label("ceqs");
            let dflt = g.label("ceqt");
            let teq = g.op(format!("icmp eq i64 {}, {}", tl_, tr))?;
            g.condbr(&teq, &same, &fail)?;
            g.place(&same);
            let vs: Vec<Var9<'_>> = g9_variants(d).into_iter().filter(|(_, _, fs)| !fs.is_empty()).collect();
            let mut lbls: Vec<String> = Vec::new();
            for _ in &vs {
                lbls.push(g.label("ceqv"));
            }
            let mut cases: Vec<String> = Vec::new();
            for ((_vn, t, _fs), l) in vs.iter().zip(&lbls) {
                cases.push(format!("i64 {}, label %{}", t.unwrap_or(0), l));
            }
            g.term(format!("switch i64 {}, label %{} [ {} ]", tl_, dflt, cases.join(" ")))?;
            g.place(&dflt);
            g.term("ret i1 true".to_string())?;
            for ((vn, _t, fs), l) in vs.iter().zip(&lbls) {
                g.place(l);
                g9_eq_conj(&mut g, &ty, &fail, *vn, fs)?;
            }
        }
    }
    g.place(&fail);
    g.term("ret i1 false".to_string())?;
    Ok(g.render(&format!("define internal i1 @pruv_ceq__{}(ptr %a, ptr %b, ptr %bud)", d.name())))
}

/// names of the user types a field of type t can contribute to `size` / structural equality: the type itself, or the
/// payload type of Option / Result (arrays, str and scalars contribute nothing)
fn g9_nominals_in(t: &str) -> Vec<String> {
    if let Some(elem) = native_vec_elem(t) {
        return vec![elem.to_string()];
    }
    if is_gopt(t) {
        let mut names = g9_nominals_in(g_arg(t));
        if t.starts_with("result<") {
            for name in g9_nominals_in(g_error(t)) {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        return names;
    }
    if crate::emit_common::is_user_key(t) {
        return vec![t.to_string()];
    }
    Vec::new()
}

/// clone / drop / node helpers of every extended type (SCC-condensed order), then the contract size / equality helpers
/// the requires checks needed (closed over the nominal field types, declaration order)
fn g9_helper_chunks(m: &mut Mod, tt: &TypeTable, vec_budget: Option<u64>) -> R<Vec<String>> {
    let mut targets: HashSet<String> = HashSet::new();
    for d in tt.ordered() {
        for (vn, _t, fs) in g9_variants(d) {
            for f in fs {
                if tt.g9_boxed.contains(&fld_label(d.name(), vn, &f.name)) {
                    targets.insert(f.ty.clone());
                }
            }
        }
    }
    let mut chunks: Vec<String> = Vec::new();
    for d in tt.ordered() {
        if !tt.g9_ext.contains(d.name()) {
            continue;
        }
        chunks.push(g9_drop_text(m, tt, d)?);
        chunks.push(g9_clone_text(m, tt, d)?);
        if targets.contains(d.name()) {
            chunks.extend(g9_node_texts(m, tt, d)?);
        }
    }
    for elem in m.vec_elems.clone() {
        chunks.extend(g9_vec_texts(m, tt, &elem, vec_budget)?);
    }
    let close = |seed: &BTreeSet<String>| -> BTreeSet<String> {
        let mut out: BTreeSet<String> = BTreeSet::new();
        let mut work: Vec<String> = seed.iter().cloned().collect();
        while let Some(n) = work.pop() {
            if out.contains(&n) {
                continue;
            }
            let d = match tt.decl(&n) {
                Some(d) => d,
                None => continue,
            };
            out.insert(n);
            for (_vn, _t, fs) in g9_variants(d) {
                for f in fs {
                    work.extend(g9_nominals_in(native_vec_elem(&f.ty).unwrap_or(&f.ty)));
                }
            }
        }
        out
    };
    let sizes = close(&m.need_size);
    let eqs = close(&m.need_eq);
    for d in &tt.decls {
        if sizes.contains(d.name()) {
            chunks.push(g9_size_text(m, tt, d)?);
        }
        if eqs.contains(d.name()) {
            chunks.push(g9_eq_text(m, tt, d)?);
        }
    }
    Ok(chunks)
}

// ---------- requires helper functions ----------
pub fn cdivrem(m: &mut Mod, is_div: bool) -> String {
    let (nm, sym) = if is_div { ("pruv_cdiv", "/") } else { ("pruv_crem", "%") };
    let z = m.string(&format!("division by zero in {}", sym));
    let o = m.string(&format!("i128 overflow in {}", sym));
    let res = if is_div { "%qq" } else { "%rr" };
    format!(
        "define internal i128 @{nm}(i128 %a, i128 %b) {{\n\
entry:\n\
\x20 %z = icmp eq i128 %b, 0\n\
\x20 br i1 %z, label %dz, label %c1\n\
dz:\n\
\x20 call void @pruv_panic_undefined(ptr {z})\n\
\x20 unreachable\n\
c1:\n\
\x20 %am = icmp eq i128 %a, {min}\n\
\x20 %bm = icmp eq i128 %b, -1\n\
\x20 %ov = and i1 %am, %bm\n\
\x20 br i1 %ov, label %of, label %c2\n\
of:\n\
\x20 call void @pruv_panic_undefined(ptr {o})\n\
\x20 unreachable\n\
c2:\n\
\x20 %q = sdiv i128 %a, %b\n\
\x20 %r = srem i128 %a, %b\n\
\x20 %neg = icmp slt i128 %r, 0\n\
\x20 %bp = icmp sgt i128 %b, 0\n\
\x20 %ra = add i128 %r, %b\n\
\x20 %rs = sub i128 %r, %b\n\
\x20 %r2 = select i1 %bp, i128 %ra, i128 %rs\n\
\x20 %rr = select i1 %neg, i128 %r2, i128 %r\n\
\x20 %qd = select i1 %bp, i128 -1, i128 1\n\
\x20 %q2 = add i128 %q, %qd\n\
\x20 %qq = select i1 %neg, i128 %q2, i128 %q\n\
\x20 ret i128 {res}\n\
}}\n",
        nm = nm,
        z = z,
        min = SIGNED_MIN128,
        o = o,
        res = res
    )
}

/// Program -> (IR text, Rust bindings text). Err is the message at the place where Python raises an exception.
pub fn gen_llvm(prog: &Program, verified: bool, trust_callers: bool, budget: u64) -> R<(String, String)> {
    gen_llvm_for(prog, verified, trust_callers, budget, AbiTarget::SysV)
}

const WIN_X64_HEADER: &str = "target datalayout = \"e-m:w-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128\"\ntarget triple = \"x86_64-pc-windows-msvc\"\n\n";

pub(crate) fn native_vec_call_elem(c: &Call) -> Option<&str> {
    if !matches!(c.name.as_str(), "vec_new" | "vec_push" | "vec_get" | "vec_at_mut") {
        return None;
    }
    let ty = if c.name == "vec_new" {
        c.ty.as_deref()?
    } else {
        let first = c.args.first()?;
        let place = if let Expr::InoutArg(arg) = first { &arg.expr } else { first };
        expr_ty(place).as_deref()?
    };
    native_vec_elem(ty)
}

fn g6_scalar(t: &str) -> bool {
    t == "i64" || t == "bool"
}

/// G6_SPEC 13.1 collections, plus the G10 File handle.
fn g6_coll_ok(t: &str) -> bool {
    use crate::gtype::GType;
    match GType::try_from_key(t) {
        Ok(g) => match g.name.as_str() {
            "vec" => g.args.first().map_or(false, |a| g6_scalar(&a.to_key())),
            "hashmap" => g.args.len() >= 2 && g6_scalar(&g.args[0].to_key()) && g6_scalar(&g.args[1].to_key()),
            _ => matches!(g.name.as_str(), "string" | "file" | "process" | "process_args" | "socket" | "listener" | "loop"),
        },
        Err(_) => false,
    }
}

/// a type that may occur in a G6 program on LLVM: no collection, or a supported one (never nested)
fn g6_type_ok(t: &str, vec_elems: &BTreeSet<String>) -> bool {
    if let Some(elem) = native_vec_elem(t) {
        return vec_elems.contains(elem);
    }
    if is_collection(t) {
        return g6_coll_ok(t);
    }
    if is_owned_result(t) {
        // The typed Result check already requires a value Ok payload.
        return !contains_collection(g_arg(t));
    }
    !contains_collection(t)
}

/// PruvError codegen/unsupported_type for the first G6 construct outside G6_SPEC 13.1, or None. Source order:
/// functions in order, the signature before the body, inside a body the earliest position.
pub fn g6_llvm_error(prog: &Program) -> Option<crate::diagnostic::PruvError> {
    use crate::foreign_shim::{walk_stmts, Node};
    crate::foreign::first_g6_use(prog)?;
    let vec_elems = recursive_vec_elements(prog);
    let err = |line: i64, col: i64| {
        crate::diagnostic::PruvError::new("codegen", line, col, "unsupported_type", G6_UNSUPPORTED_MSG, G6_UNSUPPORTED_FIX)
    };
    let rust_error = |line, col, what| {
        crate::diagnostic::PruvError::new("codegen", line, col, "unsupported_type",
            format!("{what} needs the Rust backend"), "use the Rust backend")
    };
    fn needs_rust(ty: &crate::gtype::GType) -> bool {
        matches!(ty.name.as_str(), "vec" | "hashmap")
            && ty.args.first().is_some_and(|k| k.to_key() == "string")
            || ty.args.iter().any(needs_rust)
    }
    let rust_type = |key: &str| needs_rust(&crate::gtype::GType::from_key(key));
    let funcs: HashMap<_, _> = prog.functions.iter().map(|f| (f.name.as_str(), f)).collect();
    for f in &prog.functions {
        if f.foreign.is_some() {
            if rust_type(&f.ret) {
                return Some(rust_error(f.line, f.col, "a trusted Vec<String> result"));
            }
            // G6b: catalog String and Vec<i64> results are boxed handles
            let owned = crate::gtype::GType::try_from_key(&f.ret).map(|t| crate::foreign::owned_result(&t)).unwrap_or(false);
            let ok = (matches!(f.ret.as_str(), "i64" | "bool" | "unit") || owned)
                && f.params.iter().all(|p| {
                    matches!(p.ty.as_str(), "i64" | "bool" | "array" | "array_u8" | "array_bool" | "str") && !p.take
                });
            if !ok {
                return Some(err(f.line, f.col));
            }
            continue;
        }
        if f.params.iter().any(|p| rust_type(&p.ty)) || rust_type(&f.ret) {
            return Some(rust_error(f.line, f.col, "this collection type"));
        }
        if f.params.iter().any(|p| !g6_type_ok(&p.ty, &vec_elems)) || !g6_type_ok(&f.ret, &vec_elems) {
            return Some(err(f.line, f.col));
        }
        let mut bad: Vec<(i64, i64)> = Vec::new();
        let mut rust_only = Vec::new();
        walk_stmts(&f.body, &mut |n: Node<'_>| match n {
            Node::ForMap(w) => rust_only.push((w.line, w.col, "map iteration")),
            Node::Let(l) => {
                if !g6_type_ok(&l.ty, &vec_elems) {
                    bad.push((l.line, l.col));
                }
            }
            Node::Expr(e) => {
                if matches!(e, Expr::Call(c) if native_vec_call_elem(c).is_some_and(|elem| vec_elems.contains(elem))
                    && !funcs.contains_key(c.name.as_str())) {
                    return;
                }
                if let Some(t) = expr_ty(e) {
                    if rust_type(t) { rust_only.push((e.line(), e.col(), "this collection type")); }
                    if !g6_type_ok(t, &vec_elems) {
                        bad.push((e.line(), e.col()));
                    }
                }
                if let Expr::Call(c) = e {
                    if funcs.get(c.name.as_str()).is_some_and(|callee| {
                        c.args.iter().zip(&callee.params).any(|(arg, p)| {
                            expr_ty(arg).as_deref().is_some_and(|ty| crate::gtype::key_is_vec_slice_arg(ty, &p.ty))
                        })
                    }) {
                        rust_only.push((c.line, c.col, "a borrowed vector slice argument"));
                    }
                    match c.name.as_str() {
                        "map_remove" => rust_only.push((c.line, c.col, "map_remove")),
                        "map_take" => rust_only.push((c.line, c.col, "map_take")),
                        "vec_at_mut" => rust_only.push((c.line, c.col, "vec_at_mut")),
                        "vec_clear" => rust_only.push((c.line, c.col, "vec_clear")),
                        "vec_extend_from_slice" => rust_only.push((c.line, c.col, "vec_extend_from_slice")),
                        "string_eq" => rust_only.push((c.line, c.col, "string_eq")),
                        "map_get_mut" => bad.push((c.line, c.col)),
                        "vec_reserve"
                            if !c.inst.as_ref().and_then(|i| i.args.first()).is_some_and(|a| {
                                let k = a.to_key();
                                k == "i64" || k == "bool"
                            }) =>
                        {
                            rust_only.push((c.line, c.col, "vec_reserve"))
                        }
                        _ => {}
                    }
                    if crate::foreign::is_catalog(&c.name) && !crate::foreign::is_catalog_contract_only(&c.name) {
                        if let Some(inst) = &c.inst {
                            if !inst.args.iter().all(|a| g6_scalar(&a.to_key())) {
                                bad.push((c.line, c.col));
                            }
                        }
                    }
                }
            }
        });
        if let Some(&(line, col, what)) = rust_only.iter().min() {
            return Some(rust_error(line, col, what));
        }
        if let Some(&(line, col)) = bad.iter().min() {
            return Some(err(line, col));
        }
    }
    None
}

fn is_coll_fn(f: &Function) -> bool {
    f.params.iter().any(|p| is_collection(&p.ty)) || is_collection(&f.ret)
}

/// Python `is_host_abi_fn`: a function that gets `pruv_abi_*` wrappers: legacy = every ordinary function that is not a
/// specialization or collection/extended-bearing; G4 explicit mode = additionally only the host entries (a child-public
/// function is never exported just because Pruv callers can name it). `host` = the function's host-entry flag.
pub fn is_host_abi_fn(f: &Function, tt: &TypeTable, explicit: bool, host: bool) -> bool {
    f.foreign.is_none()
        && !crate::emit_common::is_spec_fn(&f.name)
        && !is_coll_fn(f)
        && !has_ext_sig(f, tt)
        && !has_take_array(f)
        && !view_ret(f) // R-6g9-1: a borrowed result never crosses the host boundary
        && (host || !explicit)
        // an inout entry is exported only as an explicit-mode host entry (legacy inout fns stay internal)
        && (!has_inout(f) || (explicit && host))
}

/// Overlap-check library (module text after the pruv_shrink_v2 slot; set by `Mod::overlap`)
const DECLS_OVERLAP: &str = "declare { i64, i1 } @llvm.umul.with.overflow.i64(i64, i64)
declare { i64, i1 } @llvm.uadd.with.overflow.i64(i64, i64)
define internal i1 @pruv_overlap(ptr %a, i64 %an, i64 %ae, ptr %b, i64 %bn, i64 %be) nounwind {
entry:
  %an.neg = icmp slt i64 %an, 0
  %bn.neg = icmp slt i64 %bn, 0
  %neg = or i1 %an.neg, %bn.neg
  br i1 %neg, label %bad, label %len
len:
  %am = call { i64, i1 } @llvm.umul.with.overflow.i64(i64 %an, i64 %ae)
  %ab = extractvalue { i64, i1 } %am, 0
  %ao = extractvalue { i64, i1 } %am, 1
  %bm = call { i64, i1 } @llvm.umul.with.overflow.i64(i64 %bn, i64 %be)
  %bb = extractvalue { i64, i1 } %bm, 0
  %bo = extractvalue { i64, i1 } %bm, 1
  %a0 = ptrtoint ptr %a to i64
  %b0 = ptrtoint ptr %b to i64
  %ax = call { i64, i1 } @llvm.uadd.with.overflow.i64(i64 %a0, i64 %ab)
  %a1 = extractvalue { i64, i1 } %ax, 0
  %axo = extractvalue { i64, i1 } %ax, 1
  %bx = call { i64, i1 } @llvm.uadd.with.overflow.i64(i64 %b0, i64 %bb)
  %b1 = extractvalue { i64, i1 } %bx, 0
  %bxo = extractvalue { i64, i1 } %bx, 1
  %o1 = or i1 %ao, %bo
  %o2 = or i1 %o1, %axo
  %o3 = or i1 %o2, %bxo
  br i1 %o3, label %bad, label %cmp
cmp:
  %az = icmp eq i64 %ab, 0
  %bz = icmp eq i64 %bb, 0
  %z = or i1 %az, %bz
  %c1 = icmp ult i64 %a0, %b1
  %c2 = icmp ult i64 %b0, %a1
  %c = and i1 %c1, %c2
  %nz = xor i1 %z, true
  %r = and i1 %c, %nz
  ret i1 %r
bad:
  ret i1 true
}
";

/// Generation with a chosen target ABI. SysV is byte-identical to gen_llvm.
pub fn gen_llvm_for(prog: &Program, verified: bool, trust_callers: bool, budget: u64, target: AbiTarget) -> R<(String, String)> {
    gen_llvm_any(prog, verified, trust_callers, budget, target).map_err(|e| match e {
        crate::codegen::GenError::Msg(m) => m,
        crate::codegen::GenError::Diag(p) => format!("{}: {}", p.code, p.message),
    })
}

/// Python `gen_llvm`: a program that uses generics is specialized first (shared plan, mono.rs; a specialization limit is
/// a diagnostic) and lowered from the concrete program; any other program is lowered directly.
pub fn gen_llvm_any(
    prog: &Program,
    verified: bool,
    trust_callers: bool,
    budget: u64,
    target: AbiTarget,
) -> Result<(String, String), crate::codegen::GenError> {
    gen_llvm_any_ex(prog, verified, trust_callers, budget, target, "requires")
}

/// `gen_llvm_any` with the G6 boundary check mode of the shim helpers (requires|all|off, docs/G6_SPEC.md 12.3).
/// A G6 program outside the supported subset (G6_SPEC 13.1) is a codegen/unsupported_type diagnostic.
pub fn gen_llvm_any_ex(
    prog: &Program,
    verified: bool,
    trust_callers: bool,
    budget: u64,
    target: AbiTarget,
    foreign_checks: &str,
) -> Result<(String, String), crate::codegen::GenError> {
    gen_llvm_any_ex_g4(prog, None, verified, trust_callers, budget, target, foreign_checks)
}

/// `gen_llvm_any` for a checked program of explicit visibility mode (G4 A3; `g4` = None is `gen_llvm_any`).
pub fn gen_llvm_any_g4(
    prog: &Program,
    g4: Option<&crate::modules::G4>,
    verified: bool,
    trust_callers: bool,
    budget: u64,
    target: AbiTarget,
) -> Result<(String, String), crate::codegen::GenError> {
    gen_llvm_any_ex_g4(prog, g4, verified, trust_callers, budget, target, "requires")
}

/// `gen_llvm_any_ex` with the explicit-mode data (G4 A3): explicit mode always goes through the specialization plan, which
/// names every function and type by its Rust stem (Python `gen_llvm`: `first_g3_use(prog) is not None or prog.explicit`).
pub fn gen_llvm_any_ex_g4(
    prog: &Program,
    g4: Option<&crate::modules::G4>,
    verified: bool,
    trust_callers: bool,
    budget: u64,
    target: AbiTarget,
    foreign_checks: &str,
) -> Result<(String, String), crate::codegen::GenError> {
    use crate::codegen::GenError;
    if !matches!(foreign_checks, "requires" | "all" | "off") {
        return Err(GenError::Msg("foreign_checks must be requires, all or off".to_string()));
    }
    // R-6g9-1: views lower to { ptr, i64 }; borrowing structs wait for R-6g9-2 (B2)
    if let Some(TypeDecl::Struct(d)) = prog.types.iter().find(|d| crate::borrowed::borrowing_struct(d)) {
        return Err(GenError::Diag(crate::diagnostic::PruvError::new(
            "codegen", d.line, d.col, "unsupported_type",
            "borrowing structs need the Rust backend", "use the Rust backend",
        )));
    }
    if crate::typeinfo::first_g3_use(prog).is_some() || g4.is_some() {
        let plan = crate::mono::specialize_g4(prog, g4).map_err(|e| match e {
            crate::mono::MonoError::Limit(p) => GenError::Diag(p),
            crate::mono::MonoError::Internal(m) => GenError::Msg(m),
        })?;
        if let Some(e) = crate::bindings::llvm_param_abi_error(prog, g4) {
            return Err(GenError::Diag(e));
        }
        return gen_llvm_with_specializations(&plan.program, verified, trust_callers, budget, target, foreign_checks, &plan.args);
    }
    gen_llvm_concrete_ex(prog, verified, trust_callers, budget, target, foreign_checks)
}

/// The generator proper, for a CONCRETE program (a plan.program of mono.rs, or a program without G3): no type parameter
/// reaches it.
pub fn gen_llvm_concrete(prog: &Program, verified: bool, trust_callers: bool, budget: u64, target: AbiTarget) -> R<(String, String)> {
    gen_llvm_concrete_ex(prog, verified, trust_callers, budget, target, "requires").map_err(|e| match e {
        crate::codegen::GenError::Msg(m) => m,
        crate::codegen::GenError::Diag(p) => format!("{}: {}", p.code, p.message),
    })
}

fn typed_result_llvm_error(prog: &Program, specialized: &HashMap<String, Vec<crate::gtype::GType>>) -> Option<crate::diagnostic::PruvError> {
    use crate::foreign_shim::{walk_stmts, Node};
    use crate::gtype::{format_type, is_g3_key, is_value, Decls, GType};

    let decls = |name: &str| {
        prog.types.iter().find(|d| d.name() == name).map(|d| (
            d.tparams().len(),
            d.all_fields().iter().map(|f| GType::from_key(&f.ty)).collect(),
        ))
    };
    let in_type = |ty: &GType, in_decl| {
        fn reject<'a>(ty: &'a GType, in_decl: bool, specialized: &'a HashMap<String, Vec<GType>>, decls: Decls) -> Option<&'a GType> {
            if ty.has_string_error() {
                // LB8 R-6g7-1: Result<(), String> has an empty Ok slot, so only the error needs a drop path
                if in_decl || !(ty.args[0].name == "unit" || is_value(&ty.args[0], decls)) {
                    return Some(ty);
                }
            } else if ty.args.iter().any(GType::has_string_error) {
                return Some(ty);
            }
            let args = if ty.args.is_empty() {
                specialized.get(&ty.name).map_or(&[][..], Vec::as_slice)
            } else {
                &ty.args
            };
            args.iter().find_map(|arg| reject(arg, in_decl, specialized, decls))
        }
        reject(ty, in_decl, specialized, &decls).map(|hit| (format_type(hit), hit.has_string_error()))
    };
    let in_key = |key: &str, in_decl| {
        if is_g3_key(key) || specialized.contains_key(key) {
            in_type(&GType::from_key(key), in_decl)
        } else {
            None
        }
    };
    let in_expr = |e: &Expr| {
        let targs = match e {
            Expr::Ctor(x) => x.targs.as_ref(),
            Expr::Call(x) => x.targs.as_ref(),
            Expr::StructLit(x) => x.targs.as_ref(),
            Expr::EnumCtor(x) => x.targs.as_ref(),
            _ => None,
        };
        expr_ty(e).as_deref().and_then(|key| in_key(key, false))
            .or_else(|| targs?.iter().find_map(|ty| in_type(ty, false)))
            .or_else(|| match e {
                Expr::Call(x) => specialized.get(&x.name)?.iter().find_map(|ty| in_type(ty, false)),
                _ => None,
            })
    };
    fn in_clause(e: &Expr, check: &impl Fn(&Expr) -> Option<(String, bool)>) -> Option<(String, bool)> {
        check(e).or_else(|| crate::refine::kids(e).into_iter().find_map(|child| in_clause(child, check)))
    }

    let decls = prog.types.iter().filter_map(|d| {
        let ty = d.all_fields().into_iter().find_map(|f| in_key(&f.ty, true))?;
        let name = prog.explicit.as_ref().and_then(|meta| meta.tdisp.get(d.name())).map_or(d.name(), String::as_str);
        Some((d.line(), d.col(), name, ty))
    });
    let functions = prog.functions.iter().enumerate().filter_map(|(i, f)| {
        let ty = f.params.iter().find_map(|p| in_key(&p.ty, false))
            .or_else(|| in_key(&f.ret, false))
            .or_else(|| f.requires.iter().chain(&f.ensures).chain(&f.decreases).chain(&f.cost).find_map(|e| in_clause(e, &in_expr)))
            .or_else(|| {
                let mut found = None;
                walk_stmts(&f.body, &mut |node| {
                    if found.is_none() {
                        found = match node {
                            Node::Let(l) => in_key(&l.ty, false),
                            Node::Expr(e) => in_expr(e),
                            Node::ForMap(_) => None,
                        };
                    }
                });
                found
            })?;
        let name = prog.explicit.as_ref().map_or(f.name.as_str(), |meta| meta.fns[i].disp.as_str());
        Some((f.line, f.col, name, ty))
    });
    let (line, col, name, (ty, owned)) = decls.chain(functions).min_by_key(|(line, col, _, _)| (*line, *col))?;
    let detail = if owned {
        "an owned String error Result that the llvm codegen layer supports only with a value Ok type (i64, bool, a value struct or enum, or an Option of one), outside records and other generic types"
    } else {
        "a Result with a non-i64 error type, which the llvm codegen layer does not support yet"
    };
    Some(crate::diagnostic::PruvError::new(
        "codegen", line, col, "unsupported_type",
        format!("{name} uses {ty}, {detail}"),
        "use the Rust backend",
    ))
}

/// `gen_llvm_concrete` with the G6 check mode; a diagnostic (G6 outside 13.1) is `GenError::Diag`.
pub fn gen_llvm_concrete_ex(
    prog: &Program,
    verified: bool,
    trust_callers: bool,
    budget: u64,
    target: AbiTarget,
    foreign_checks: &str,
) -> Result<(String, String), crate::codegen::GenError> {
    gen_llvm_with_specializations(prog, verified, trust_callers, budget, target, foreign_checks, &HashMap::new())
}

pub fn gen_llvm_with_specializations(
    prog: &Program,
    verified: bool,
    trust_callers: bool,
    budget: u64,
    target: AbiTarget,
    foreign_checks: &str,
    specialized: &HashMap<String, Vec<crate::gtype::GType>>,
) -> Result<(String, String), crate::codegen::GenError> {
    use crate::codegen::GenError;
    if !matches!(foreign_checks, "requires" | "all" | "off") {
        return Err(GenError::Msg("foreign_checks must be requires, all or off".to_string()));
    }
    // R-6g9-1: views lower to { ptr, i64 }; borrowing structs wait for R-6g9-2 (B2)
    if let Some(TypeDecl::Struct(d)) = prog.types.iter().find(|d| crate::borrowed::borrowing_struct(d)) {
        return Err(GenError::Diag(crate::diagnostic::PruvError::new(
            "codegen", d.line, d.col, "unsupported_type",
            "borrowing structs need the Rust backend", "use the Rust backend",
        )));
    }
    if let Some(e) = crate::bindings::owned_collection_abi_error(prog, None) {
        return Err(GenError::Diag(e));
    }
    if let Some((line, col)) = prog.contract_vec_slice {
        return Err(GenError::Diag(crate::diagnostic::PruvError::new(
            "codegen", line, col, "unsupported_type",
            "a borrowed vector slice argument needs the Rust backend", "use the Rust backend",
        )));
    }
    if let Some(e) = typed_result_llvm_error(prog, specialized) {
        return Err(GenError::Diag(e));
    }
    if let Some(e) = g6_llvm_error(prog) {
        return Err(GenError::Diag(e));
    }
    gen_llvm_body(prog, verified, trust_callers, budget, target, foreign_checks).map_err(decode_llvm_error)
}

fn decode_llvm_error(s: String) -> crate::codegen::GenError {
    use crate::codegen::GenError;
    if let Some(rest) = s.strip_prefix(E2_DIAG_TAG) {
        let mut parts = rest.splitn(3, ':');
        let line = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let col = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        return GenError::Diag(crate::diagnostic::PruvError::new(
            "codegen", line, col, "unsupported_type", parts.next().unwrap_or(""), "use the Rust backend",
        ));
    }
    match s.strip_prefix(G6_DIAG_TAG) {
        Some(rest) => {
            let mut it = rest.split(':');
            let line: i64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
            let col: i64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
            GenError::Diag(crate::diagnostic::PruvError::new(
                "codegen",
                line,
                col,
                "unsupported_type",
                G6_UNSUPPORTED_MSG,
                G6_UNSUPPORTED_FIX,
            ))
        }
        None => match s
            .strip_prefix(G9_DIAG_TAG)
            .map(|r| (r, "runtime requires"))
            .or_else(|| s.strip_prefix(G9_COST_DIAG_TAG).map(|r| (r, "kept cost clause")))
        {
            // G9 A3: FnGen.g9_unsupported (a contract operand the LLVM backend cannot evaluate); LG15 (G-LG12-5):
            // the message names the clause being evaluated
            Some((rest, clause)) => {
                let mut it = rest.splitn(3, ':');
                let line: i64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
                let col: i64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
                let what = it.next().unwrap_or("");
                GenError::Diag(crate::diagnostic::PruvError::new(
                    "codegen",
                    line,
                    col,
                    "unsupported_type",
                    &format!("{} is not supported in a {} by the llvm codegen layer yet", what, clause),
                    "compare tags with is_variant and fields with variant_field, or move the check into the body",
                ))
            }
            None => match s.strip_prefix(EQR_DIAG_TAG) {
                Some(rest) => {
                    let mut it = rest.splitn(3, ':');
                    let line: i64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
                    let col: i64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
                    let what = it.next().unwrap_or("");
                    GenError::Diag(crate::diagnostic::PruvError::new(
                        "codegen",
                        line,
                        col,
                        "unsupported_type",
                        &format!("eq_range is not supported by the llvm codegen layer for this {} yet", what),
                        "compare the elements in a loop, or use the Rust backend",
                    ))
                }
                None => match s.strip_prefix(CPR_DIAG_TAG) {
                    Some(rest) => {
                        let mut it = rest.splitn(4, ':');
                        let builtin = it.next().unwrap_or("");
                        let line: i64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
                        let col: i64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
                        let what = it.next().unwrap_or("");
                        GenError::Diag(crate::diagnostic::PruvError::new(
                            "codegen",
                            line,
                            col,
                            "unsupported_type",
                            &format!("{builtin} is not supported by the llvm codegen layer for this {} yet", what),
                            "copy the elements in a loop, or use the Rust backend",
                        ))
                    }
                    None => GenError::Msg(s),
                },
            },
        },
    }
}

fn gen_llvm_body(
    prog: &Program,
    verified: bool,
    trust_callers: bool,
    budget: u64,
    target: AbiTarget,
    foreign_checks: &str,
) -> R<(String, String)> {
    fn collect_par(stmts: &[Stmt], names: &mut BTreeSet<String>) {
        for stmt in stmts {
            match stmt {
                Stmt::Par(p) => for call in &p.calls {
                    if let Expr::Call(c) = call { names.insert(c.name.clone()); }
                },
                Stmt::If(x) => { collect_par(&x.then, names); collect_par(&x.els, names); }
                Stmt::While(x) => collect_par(&x.body, names),
                Stmt::ForMap(x) => collect_par(&x.body, names),
                Stmt::Match(x) => for arm in &x.arms { collect_par(&arm.body, names); },
                _ => {}
            }
        }
    }
    let mut par_targets = BTreeSet::new();
    for f in &prog.functions { collect_par(&f.body, &mut par_targets); }
    let is_g6 = crate::foreign::first_g6_use(prog).is_some();
    if let Some(e) = crate::typeinfo::g2_unimplemented_error(prog, "codegen", None, &["array_u8", "array_bool", "str"], None) {
        // G2 B2: str also opens the generation layer
        return Err(format!("unsupported_type: {}", e.message));
    }
    // G4 A3: the identities behind the Rust names (panic / enum texts); cleared again when the generator returns
    struct DispGuard;
    impl Drop for DispGuard {
        fn drop(&mut self) {
            crate::emit_common::set_type_disp(HashMap::new());
        }
    }
    let _disp_guard = DispGuard;
    crate::emit_common::set_type_disp(prog.explicit.as_ref().map(|x| x.tdisp.clone()).unwrap_or_default());
    // G9: SCC-condensed order and the extended / boxed metadata (identical to TypeTable::new otherwise)
    let tt = new_tt(prog);
    let mut m = Mod::new();
    let vec_elems = recursive_vec_elements(prog);
    m.owned_collection_fields = prog.types.iter().any(|d| {
        d.all_fields().iter().any(|field| crate::gtype::key_is_collection(&field.ty))
    });
    m.foreign_requires = foreign_checks != "off";
    m.vec_get_checked = !verified || foreign_checks == "all" || prog.functions.iter().any(|f| f.foreign.is_some());
    m.sym_pre = sym_prefix(prog.pkg_name.as_deref());
    m.resize_params = crate::check::resize_params(prog);
    m.storage = plan_storage(prog, verified);
    m.stack = crate::stack::plan_stack(prog, &m.storage, verified);
    let narrowed = m.storage.entries.iter().any(|(_, w)| *w != "i64");
    let explicit = prog.explicit.is_some();
    m.explicit = explicit;
    // G4 A3: per function (parallel to prog.functions): host entry flag; the display identity map
    let hosts: Vec<bool> = match &prog.explicit {
        Some(x) => x.fns.iter().map(|mk| mk.host_entry).collect(),
        None => Vec::new(),
    };
    if let Some(x) = &prog.explicit {
        for (f, mk) in prog.functions.iter().zip(x.fns.iter()) {
            m.fn_disp.entry(f.name.clone()).or_insert_with(|| mk.disp.clone());
        }
    }
    let mut funcs: HashMap<String, &Function> = HashMap::new();
    for f in &prog.functions {
        funcs.insert(f.name.clone(), f);
    }
    let mut chunks: Vec<String> = Vec::new();
    // G4 A3 (docs/G4_SPEC.md 4, same policy as codegen.Gen): the functions that get a checked `__pruv_entry`.
    // Verified: only host entries with a requires (none under trust_callers); safe mode: every function with a requires,
    // also under trust_callers (it removes only the host-boundary checks)
    let checked_entry = |f: &Function, host: bool| -> bool {
        !f.requires.is_empty() && f.foreign.is_none() && (!verified || (host && !trust_callers))
    };
    let need_cdiv = if explicit {
        prog.functions.iter().enumerate().any(|(i, f)| checked_entry(f, hosts.get(i).copied().unwrap_or(false)))
    } else {
        // G5 A3: an inout entry checks also under trust_callers
        prog.functions.iter().any(|f| {
            !f.requires.is_empty() && f.foreign.is_none() && (!trust_callers || (!verified && has_inout(f)))
        })
    };
    let need_cdiv = need_cdiv || (!verified && prog.functions.iter().any(|f| !f.requires.is_empty() && par_targets.contains(&f.name)));
    let cost_used = !verified && prog.functions.iter().any(|f| f.cost.is_some());
    if need_cdiv || cost_used {
        chunks.push(cdivrem(&mut m, true));
        chunks.push(cdivrem(&mut m, false));
    }
    for (fi, f) in prog.functions.iter().enumerate() {
        if f.foreign.is_some() {
            // G6 A3: a trusted rust fn has no body; its calls go to the shim adapter
            continue;
        }
        // G6 A3: collection-bearing functions are internal (no ABI wrapper, no binding); G9 A3: so are functions with
        // an extended type in their signature
        let coll = is_coll_fn(f) || is_internal_fn(f, &tt);
        let mut g = FnGen::new(&mut m, &tt, &funcs, f, verified);
        g.body()?;
        let params = ir_params(f, &tt, &g.m.resize_params);
        let cost = g.cost;
        let body = g.render(&format!(
            "define internal {} @{}__pruv_body({})",
            ir_ret(&tt, f),
            mangle(&f.name),
            sig_text(&params, Some(f))
        ));
        if cost { chunks.push(entry_of(&mut m, &tt, &funcs, f, trust_callers, budget, true, false)?); }
        chunks.push(body);
        if !verified && !f.requires.is_empty() && par_targets.contains(&f.name) {
            chunks.push(entry_of(&mut m, &tt, &funcs, f, false, budget, false, false)?.replace(
                &format!("@{}__pruv_entry(", mangle(&f.name)),
                &format!("@{}__pruv_par_chk(", mangle(&f.name))));
        }
        if explicit {
            // G4 A3: bodies and checked wrappers are internal; only a host entry gets pruv_abi_* (and its typed variant).
            // The host boundary calls the checked wrapper unless trust_callers skips the check (or there is no requires).
            let host = hosts.get(fi).copied().unwrap_or(false);
            let checked = checked_entry(f, host);
            if checked {
                chunks.push(entry_of(&mut m, &tt, &funcs, f, false, budget, false, verified)?);
            }
            if is_host_abi_fn(f, &tt, true, host) {
                let abi = crate::abi::abi_of_target(
                    f,
                    &tt,
                    target,
                    &mut m,
                    if checked && !trust_callers { "entry" } else { "body" },
                )?;
                chunks.push(prefix_abi_defs(&abi, &m.sym_pre));
            }
            continue;
        }
        if !(coll && verified) {
            // verified callers call the body; the entry only serves safe-mode callers
            // G5 A3: the entry of an inout function is internal and always checked (trust_callers does not waive it)
            chunks.push(entry_of(
                &mut m,
                &tt,
                &funcs,
                f,
                (trust_callers && !has_inout(f)) || f.requires.is_empty(),
                budget,
                false,
                verified,
            )?);
        }
        if !crate::emit_common::is_spec_fn(&f.name) && !coll {
            // G3: a generic specialization has no ABI symbol
            let abi = abi_of(f, &tt, target, &mut m)?;
            chunks.push(prefix_abi_defs(&abi, &m.sym_pre));
        }
    }
    if !tt.g9_ext.is_empty() || !m.vec_elems.is_empty() || !m.need_size.is_empty() || !m.need_eq.is_empty() {
        // G9 A3: clone / drop / node helpers, then the contract helpers above
        chunks.extend(g9_helper_chunks(&mut m, &tt, (foreign_checks == "all").then_some(budget))?);
    }
    // G9 A3: buffer fields count as uses of the buffer runtime declarations
    let has_str = crate::typeinfo::uses_str(prog, !verified) || type_has_field(prog, "str"); // G2 B2
    let g9_u8 = type_has_field(prog, "array_u8") || type_has_field(prog, "array_bool");
    // G6 A3: bridge marker and the shim symbols used, sorted by symbol
    let mut g6decls = String::new();
    if m.g6_bridge || !m.g6_decls.is_empty() {
        g6decls.push_str(&G6_BRIDGE_DECL.replacen('@', &format!("@{}", m.sym_pre), 1));
        for line in m.g6_decls.values() {
            g6decls.push_str(line);
            g6decls.push('\n');
        }
    }
    let memmove = chunks.iter().any(|chunk| chunk.contains("call void @llvm.memmove.p0.p0.i64("));
    let mut ir = format!(
        "{}{}\n{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}\n{}",
        if target == AbiTarget::WinX64 { WIN_X64_HEADER } else { "" },
        m.render_strings(),
        type_defs(&tt),
        if prog.types.is_empty() { "" } else { "\n" },
        DECLS,
        // G2 A5: DECLS_U8 if u8 or bool is used, and DECLS_BOOL after it if bool is used
        // G2 B2: DECLS_U8 if u8, bool or str is used, and DECLS_STR at the very end if str is used
        // G6 A3: DECLS_U8 also when string_to_str copies through pruv_copy_v2
        // G9 A3: DECLS_U8 also for a [u8] / [bool] buffer field
        if uses_array_type(prog, "array_u8") || uses_array_type(prog, "array_bool") || has_str || m.g6_v2 || g9_u8 || narrowed {
            DECLS_U8
        } else {
            ""
        },
        if narrowed { "declare noalias ptr @pruv_alloc_narrow(i64, i64, i64, i64) nounwind\n" } else { "" },
        if uses_array_type(prog, "array_bool") { DECLS_BOOL } else { "" },
        if has_str { DECLS_STR } else { "" },
        if m.uses_repr { DECLS_REPR } else { "" }, // G3
        format!("{}{}",
            if !tt.g9_boxed.is_empty() || !m.vec_elems.is_empty() { DECLS_NODE } else { "" },
            if !m.vec_elems.is_empty() && !m.overlap { "declare { i64, i1 } @llvm.umul.with.overflow.i64(i64, i64)\n" } else { "" }),
        // P2 batch 4: llvm.bswap declarations of the widths used, ascending
        m.bswap.iter().map(|w| format!("declare i{w} @llvm.bswap.i{w}(i{w})\n", w = w)).collect::<String>(),
        if m.shrink { "declare ptr @pruv_shrink_v2(ptr, i64, i64, i64, i64) nounwind\n" } else { "" },
        if m.overlap { DECLS_OVERLAP } else { "" },
        if m.fill { "declare void @pruv_fill_v2(ptr nocapture, i64, i64, i64) nounwind\n" } else { "" },
        if m.tail_i64 { DECLS_TAIL } else { "" },
        if m.tail_u8 { DECLS_TAIL_U8 } else { "" },
        if m.tail_i64 || m.tail_u8 { DECLS_MINMAX } else { "" },
        if m.resize_i64 { DECLS_RESIZE } else { "" },
        if m.resize_u8 { DECLS_RESIZE_U8 } else { "" },
        if m.resize_narrow { DECLS_RESIZE_NARROW } else { "" },
        if m.raw_regrow || m.regrow_i64 || m.regrow_u8 || m.regrow_narrow { DECL_REGROW_V2 } else { "" },
        if m.regrow_i64 { DECLS_REGROW } else { "" },
        if m.regrow_u8 { DECLS_REGROW_U8 } else { "" },
        if m.regrow_narrow { DECLS_REGROW_NARROW } else { "" },
        format!("{}{}{}",
            if memmove { "declare void @llvm.memmove.p0.p0.i64(ptr nocapture writeonly, ptr nocapture readonly, i64, i1 immarg)\n" } else { "" },
            if m.memcmp { "declare i32 @memcmp(ptr nocapture, ptr nocapture, i64) nounwind readonly\n" } else { "" }, // P17a
            if m.memchr { "declare ptr @memchr(ptr, i32, i64) nounwind readonly\n" } else { "" }), // LB6 MC
        g6decls,
        chunks.join("\n")
    );
    if m.vec_get_used {
        ir.push_str("declare void @pruv_panic_vec_get_v1(i64) noreturn cold nounwind\n");
    }
    if !m.vec_elems.is_empty() && m.foreign_requires {
        ir.push_str("declare void @pruv_panic_vec_push_v1(i64) noreturn cold nounwind\n");
    }
    if m.uses_utf8_valid {
        ir.push_str(include_str!("../../../../pruvc/utf8_valid.ll"));
    }
    if m.uses_utf8_panic {
        ir.push_str("declare void @pruv_panic_utf8(i64) noreturn cold nounwind\n");
    }
    if !m.par_trampolines.is_empty() {
        ir = format!("%pruv_g7_job = type {{ ptr, ptr }}\n{}\n{}", m.par_types.join("\n"), ir);
        ir.push_str("declare void @pruv_g7_v1_run(ptr, i64) nounwind\n");
        ir.push_str(&m.par_trampolines.join("\n"));
    }
    let mut shim = if is_g6 && (!m.g6_decls.is_empty() || m.g6_bridge || vec_elems.is_empty()) {
        Some(crate::foreign_shim::build_shim(prog, verified, trust_callers, budget, foreign_checks, m.g6_take, &m.g6_unbox.iter().cloned().collect::<Vec<String>>(), &m.sym_pre)?)
    } else {
        None
    };
    if prog.functions.iter().any(|f| crate::foreign::first_par(&f.body).is_some()) {
        shim.get_or_insert_with(String::new).push_str(&crate::parallel_shim::build_shim());
    }
    if cost_used {
        ir.push_str("declare void @pruv_panic_cost() noreturn cold nounwind\n");
        ir.push_str("declare { i128, i1 } @llvm.uadd.with.overflow.i128(i128, i128)\n");
        shim.get_or_insert_with(String::new).push_str(r#"
#[no_mangle]
pub extern "C" fn pruv_panic_cost() -> ! {
    eprintln!("pruv: cost exceeded");
    ::std::process::exit(101);
}
"#);
    }
    // P3 batch 2: !range metadata nodes, last in the module
    if !m.range_nodes.is_empty() {
        ir.push('\n');
        for (i, body) in m.range_nodes.iter().enumerate() {
            ir.push_str(&format!("!{} = !{{{}}}\n", i, body));
        }
    }
    let bindings = crate::bindings::gen_bindings_with(prog, shim.as_deref(), verified)?;
    Ok((crate::llvm_inline::llvm_inline(ir), bindings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mangle_and_types() {
        assert_eq!(mangle("a_b1"), "a_b1");
        assert_eq!(mangle("a.é"), "a$002e$00e9");
        assert_eq!(lty("bool"), "i1");
        assert_eq!(lty("Point"), "%pruv_t_Point");
        assert_eq!(st("i64", true), "i128");
        assert_eq!(st("Point", true), "%pruv_c_Point");
    }

    #[test]
    fn strings_render() {
        let mut m = Mod::new();
        assert_eq!(m.string("a\"b"), "@.str.0");
        assert_eq!(m.string("a\"b"), "@.str.0");
        assert_eq!(m.string("é"), "@.str.1");
        assert_eq!(
            m.render_strings(),
            "@.str.0 = private unnamed_addr constant [4 x i8] c\"a\\22b\\00\"\n@.str.1 = private unnamed_addr constant [3 x i8] c\"\\C3\\A9\\00\"\n"
        );
    }

    #[test]
    fn fn_gen_labels_and_guard() {
        let tt = TypeTable::new(&[]);
        let funcs: HashMap<String, &Function> = HashMap::new();
        let f = Function { borrow_from: None, borrowed_result: false,
            line: 1,
            col: 1,
            name: "f".into(),
            params: vec![],
            ret: "unit".into(),
            effects: vec![],
            requires: vec![],
            ensures: vec![],
            body: vec![],
            ret_refine: None,
            tparams: vec![],
            foreign: None,
            cost: None, decreases: None, small_bound: None, x_auto: None, opaque: false, pred: false, attribute: None,
        };
        let mut m = Mod::new();
        let mut g = FnGen::new(&mut m, &tt, &funcs, &f, false);
        g.place("start");
        let c = g.op("icmp eq i64 1, 1".to_string()).unwrap();
        g.guard(&c, Fail::Panic1("pruv_panic_arith", "x".to_string()), false).unwrap();
        let text = g.render("define internal void @f__pruv_body()");
        assert!(text.contains("  %t1 = icmp eq i64 1, 1\n  br i1 %t1, label %ok.1, label %fail.2\nfail.2:\n"));
    }

    fn opt_prog() -> Program {
        Program {
            types: vec![],
            functions: vec![Function { borrow_from: None, borrowed_result: false,
                line: 1,
                col: 1,
                name: "f".into(),
                params: vec![],
                ret: "option".into(),
                effects: vec![],
                requires: vec![],
                ensures: vec![],
                body: vec![],
                ret_refine: None,
                tparams: vec![],
                foreign: None,
                cost: None, decreases: None, small_bound: None, x_auto: None, opaque: false, pred: false, attribute: None,
            }],
            refined: false,
            g9: None,
            explicit: None,
            uses_inout: None, contract_vec_slice: None,
            raw_entries: None,
            pkg_name: None,
        }
    }

    #[test]
    fn gen_llvm_for_targets() {
        let p = opt_prog();
        let (sys, b1) = gen_llvm(&p, false, false, 1000).unwrap_or_else(|e| panic!("{}", e));
        let (sys2, _) = gen_llvm_for(&p, false, false, 1000, AbiTarget::SysV).unwrap();
        assert_eq!(sys, sys2);
        assert!(sys.contains("define { i64, i64 } @pruv_abi_f()"));
        assert!(!sys.contains("target triple"));
        let (win, b2) = gen_llvm_for(&p, false, false, 1000, AbiTarget::WinX64).unwrap();
        let mut lines = win.lines();
        assert!(lines.next().unwrap().starts_with("target datalayout = \"e-m:w-p270:32:32"));
        assert_eq!(lines.next().unwrap(), "target triple = \"x86_64-pc-windows-msvc\"");
        assert_eq!(lines.next().unwrap(), "");
        assert!(win.contains("define void @pruv_abi_f(ptr sret({ i64, i64 }) align 8 %sret) {"));
        assert_eq!(b1, b2);
    }

    #[test]
    fn cdivrem_text() {
        let mut m = Mod::new();
        let t = cdivrem(&mut m, true);
        assert!(t.starts_with("define internal i128 @pruv_cdiv(i128 %a, i128 %b) {\nentry:\n  %z = icmp eq i128 %b, 0\n"));
        assert!(t.contains("icmp eq i128 %a, -170141183460469231731687303715884105728\n"));
        assert!(t.ends_with("  ret i128 %qq\n}\n"));
    }
}

/// P3 (FnGen::tail_fill): e has no call and does not name x
fn tail_pure(e: &Expr, x: &str) -> bool {
    match e {
        Expr::IntLit(_) | Expr::BoolLit(_) => true,
        Expr::Var(v) => v.name != x,
        Expr::Unary(u) => tail_pure(&u.operand, x),
        Expr::Binary(b) => tail_pure(&b.left, x) && tail_pure(&b.right, x),
        Expr::Index(i) => tail_pure(&i.array, x) && tail_pure(&i.index, x),
        _ => false,
    }
}
