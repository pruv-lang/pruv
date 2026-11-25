"""Pruv -> Rust.  gen_rust(prog, verified=True, trust_callers=False, budget=1 << 30)

Contracts are erased from function bodies; they are only used (a) by the verifier and
(b) to build the runtime `requires` check of the public entry point.

Modes (two independent axes):
  verified=True   Output of a program the verifier proved: array accesses are
                  `get_unchecked` (unsafe), arithmetic is plain Rust.
  verified=False  (--no-verify, "safe mode") No proof exists, so nothing unsafe is emitted
                  and nothing wraps silently: array reads/writes go through pruv_get/pruv_set
                  (panic unless 0 <= i < len), + - * / % and unary - go through
                  pruv_add/pruv_sub/pruv_mul/pruv_div/pruv_rem/pruv_neg (+ - * and unary -
                  are checked_*, panic on overflow), alloc(n, v) goes through
                  pruv_alloc (panic unless 0 <= n <= 2^40).  / and % use the same domain
                  the verifier proves (verify.py): divisor > 0 and dividend >= 0,
                  otherwise panic "division outside domain: ...".
  trust_callers=False (default)  Functions with `requires` are emitted twice; the public
                  entry checks every requires at run time and panics with
                  "pruv: requires violated: <fn>: <expr>" before running the body.
  trust_callers=True  The public entry calls the body without checks (for benchmarking).

Generated names (reserved: the parser rejects any user identifier that starts with `pruv_`
or contains `__`, error code reserved_name, so user names never collide with these):
  f                 public function, same name/params/return type as the Pruv function
                    (`#[inline]` wrapper when f has requires or is a public function with an
                    array / str parameter, otherwise the real body).  Every public wrapper first
                    checks each array / str parameter length <= 2^40 (TIDY T0 step 4,
                    "pruv: invalid representation: n (array length)" / "(str length)"), also
                    under trust_callers, then the requires.
  f__pruv_body       private unchecked body, only for those wrapped functions (Gen.body_named).
                    Verified mode: calls inside Pruv code go straight to f__pruv_body
                    (the caller proved requires and the length bound).  Safe mode: inner
                    calls go to the checked entry f.
  pruv_*             helper functions emitted once at the top of the file (only the ones
                    needed): pruv_idx pruv_oob pruv_get pruv_set pruv_ovf pruv_add pruv_sub
                    pruv_mul pruv_div pruv_rem pruv_neg pruv_alloc (safe mode) and
                    pruv_ud pruv_cidx pruv_cadd pruv_csub pruv_cmul pruv_cneg pruv_cdiv pruv_crem
                    (requires evaluation; pruv_cband pruv_cbor pruv_cbxor only when a contract uses band/bor/bxor).
                    Local temporaries pruv_* are also reserved.
                    [u8] programs also get pruv_elem_oor pruv_get_u8 pruv_set_u8 pruv_alloc_u8
                    (safe mode) and pruv_cidx_u8 (requires evaluation).

Run-time requires evaluation uses i128 (contracts are mathematical integers).  Contract
/ and % are Euclidean (div_euclid/rem_euclid, as z3 Int).  Anything undefined there
(division by zero, index out of range, i128 overflow, max/min over an empty range,
some_val of None, ...) panics with "pruv: requires undefined: ...".

G1 (structs/enums): user types are `Copy` and emitted by emit_public_types(prog) at the top of
the file.  Contract evaluation uses mirror types `pruv_contract__Name` (every i64 leaf is i128,
Option/Result carry i128) and converters `pruv_to_c__Name(value)` (recursive; an enum converts
only its active variant), so contract arithmetic never goes through i64.
Standard types are generally qualified so user types named Vec etc. cannot collide.
View wrappers use the prelude's reserved Option and Result names.

G3 (generics): gen_rust first builds the shared specialization plan (mono.py) and generates from the CONCRETE program
it contains, so there is no generic Rust. Generic function specializations are private fns named `pruv_g3_f_...`
(with `__pruv_body` + a private checked entry when they have `requires`); generic nominal applications are ordinary
`pub struct/enum pruv_g3_t_...` with the usual derives. Option<X> / Result<X> (X != i64) are
`::std::option::Option<R(X)>` / `::std::result::Result<R(X), i64>`; their constructors are written with explicit
turbofish (`None::<R(X)>`, `Ok::<R(X), i64>(..)`). Contract mirrors are recursive: C(Option<X>) = Option<C(X)>,
C(Result<X>) = Result<C(X), i128>. A program without generics is generated directly, byte for byte as before G3.

G9 (recursion, recursive types, aggregate buffer fields; docs/G9_SPEC.md 3, 4, 6, 12): recursion is an ordinary Rust call.
An EXTENDED type (prog.g9["extended"]) derives `Clone, Debug` and never Copy; a recursive edge (prog.g9["boxed"]) is
`::std::boxed::Box<Child>`; buffer fields are Vec<i64>/Vec<u8>/Vec<bool>/String. Value contexts:
  read parameter / payload view  `&T`     (a call argument borrows: `f(&local)`, `f(param)`, `f(&(fresh()))`)
  owning position                 let / assignment / constructor field / return: a named value or a field path is
                                  cloned (`::std::clone::Clone::clone(&x)`), a constructor / call result moves; a
                                  constructor stores a recursive edge as `Box::new(..)`
  match on an extended enum       `match <view> { E::V(a, b) => { let a: i64 = *a; let b: &T = b; .. } }`
A function with an extended type in its signature is private (and so is its checked entry wrapper). Runtime requires use
`size` and `==` on extended values through generated helpers `pruv_csize__Name` / `pruv_ceq__Name` (declaration order,
only the needed ones) over the REAL values: nothing is converted eagerly, only active variants are visited, one budget
unit per node, i128 checked additions; exhaustion -> `pruv: requires undefined: ...`. Types are emitted in the order of
the SCC condensation of the field graph (the same as before for acyclic programs).

G4 A2 (explicit visibility mode, docs/G4_SPEC.md 4, 6; Program.explicit): the program always comes from mono.specialize, which
renames every function and type to its Rust stem (modules.encode_stem; root host entries and root `pub` types keep their
source spelling) and records `pub` / `host_entry` / `disp` (the source-qualified identity used in panic text and manifests).
Only host entries are `pub fn`; source-`pub` types are `pub` Rust types, private ones are private items. Wrappers: verified
mode emits a checked wrapper only for host entries with a requires (none under trust_callers) and Pruv calls invoke
`<stem>__pruv_body`; safe mode emits a private checked wrapper `<stem>` for EVERY function with a requires (and Pruv calls
go through it), under trust_callers the pub entry is unchecked and `<entry>__pruv_chk` is the checked one Pruv callers use.
"""
from collections import namedtuple
from vast import *
from typeinfo import (TypeTable, is_user_type, is_array, uses_array_type, uses_str, is_collection, VIEW_BASES,
                      view_base, view_payload,
                      dependencies, is_owned_field_type, is_nested_collection_field, is_string_record_map,
                      field_place, is_vec_slice_arg, is_owned_result, is_owned_value, borrowing_struct, borrowing_copy)
from foreign import CATALOG
from inline_policy import PIN_INLINE, PIN_ALWAYS, inline_policy
from check import ARRAY_GROW_BUILTINS, BIT_BUILTINS, array_resize_params, array_resize_args, same_place
from refine import _kids as expr_kids, _kids, _stmt_names, _expr_names


def _moves_name(node, names):
    """G-DB16-1: does the syntax tree `node` (a statement list, statement or expression) hold `move x` of a name in names"""
    if isinstance(node, (list, tuple)):
        return any(_moves_name(x, names) for x in node)
    if not hasattr(node, "__dataclass_fields__") or isinstance(node, Type):
        return False
    if isinstance(node, Move) and node.name in names:    # LB9 (R-6g8-4): `move b.f` of a struct binder too
        return True
    return any(_moves_name(getattr(node, f), names) for f in node.__dataclass_fields__)


def match_moves_binder(st):
    """G-DB16-1: an arm of the user-enum match `st` moves one of its own binders (the checker only admits that for an
    array binder of a plain owned scrutinee that no arm names): the match consumes its scrutinee"""
    return any(a.enum_name is not None and a.binders and _moves_name(a.body, set(a.binders)) for a in st.arms)


ALLOW = ("#[allow(unused_mut, unused_variables, unused_parens, non_snake_case, non_camel_case_types, "
         "dead_code, unreachable_code, unreachable_patterns, "
         "unconditional_panic, arithmetic_overflow)]")

SAFE_PRELUDE = """\
#[allow(dead_code)]
#[cold]
#[inline(never)]
fn pruv_oob(i: i64, len: usize) -> ! { panic!("pruv: index out of bounds: {} (len {})", i, len) }
#[allow(dead_code)]
#[cold]
#[inline(never)]
fn pruv_ovf(what: &str) -> ! { panic!("pruv: arithmetic error: {}", what) }
#[allow(dead_code)]
#[inline]
fn pruv_idx(i: i64, len: usize) -> usize { if i >= 0 && (i as u64) < (len as u64) { i as usize } else { pruv_oob(i, len) } }
#[allow(dead_code)]
#[inline]
fn pruv_get(a: &[i64], i: i64) -> i64 { a[pruv_idx(i, a.len())] }
#[allow(dead_code)]
#[inline]
fn pruv_set(a: &mut [i64], i: i64, v: i64) { let k = pruv_idx(i, a.len()); a[k] = v; }
#[allow(dead_code)]
#[inline]
fn pruv_add(a: i64, b: i64) -> i64 { match a.checked_add(b) { Some(v) => v, None => pruv_ovf("overflow in +") } }
#[allow(dead_code)]
#[inline]
fn pruv_sub(a: i64, b: i64) -> i64 { match a.checked_sub(b) { Some(v) => v, None => pruv_ovf("overflow in -") } }
#[allow(dead_code)]
#[inline]
fn pruv_mul(a: i64, b: i64) -> i64 { match a.checked_mul(b) { Some(v) => v, None => pruv_ovf("overflow in *") } }
#[allow(dead_code)]
#[inline]
fn pruv_div(a: i64, b: i64) -> i64 { if b > 0 && a >= 0 { a / b } else { pruv_ovf("division outside domain: need divisor > 0 and dividend >= 0") } }
#[allow(dead_code)]
#[inline]
fn pruv_rem(a: i64, b: i64) -> i64 { if b > 0 && a >= 0 { a % b } else { pruv_ovf("division outside domain: need divisor > 0 and dividend >= 0") } }
#[allow(dead_code)]
#[inline]
fn pruv_neg(a: i64) -> i64 { match a.checked_neg() { Some(v) => v, None => pruv_ovf("overflow in unary -") } }
#[allow(dead_code)]
#[inline]
fn pruv_alloc(n: i64, v: i64) -> ::std::vec::Vec<i64> { if n < 0 || n > (1i64 << 40) { pruv_ovf("alloc length outside 0..2^40") } vec![v; n as usize] }
"""

CONTRACT_PRELUDE = """\
#[allow(dead_code)]
#[cold]
#[inline(never)]
fn pruv_ud(what: &str) -> ! { panic!("pruv: requires undefined: {}", what) }
#[allow(dead_code)]
#[inline]
fn pruv_cidx(a: &[i64], i: i128) -> i128 { if i >= 0 && i < (a.len() as i128) { a[i as usize] as i128 } else { pruv_ud("array index out of range") } }
#[allow(dead_code)]
#[inline]
fn pruv_cadd(a: i128, b: i128) -> i128 { a.checked_add(b).unwrap_or_else(|| pruv_ud("i128 overflow in +")) }
#[allow(dead_code)]
#[inline]
fn pruv_csub(a: i128, b: i128) -> i128 { a.checked_sub(b).unwrap_or_else(|| pruv_ud("i128 overflow in -")) }
#[allow(dead_code)]
#[inline]
fn pruv_cmul(a: i128, b: i128) -> i128 { a.checked_mul(b).unwrap_or_else(|| pruv_ud("i128 overflow in *")) }
#[allow(dead_code)]
#[inline]
fn pruv_cneg(a: i128) -> i128 { a.checked_neg().unwrap_or_else(|| pruv_ud("i128 overflow in unary -")) }
#[allow(dead_code)]
#[inline]
fn pruv_cdiv(a: i128, b: i128) -> i128 { if b == 0 { pruv_ud("division by zero in /") } a.checked_div_euclid(b).unwrap_or_else(|| pruv_ud("i128 overflow in /")) }
#[allow(dead_code)]
#[inline]
fn pruv_crem(a: i128, b: i128) -> i128 { if b == 0 { pruv_ud("division by zero in %") } a.checked_rem_euclid(b).unwrap_or_else(|| pruv_ud("i128 overflow in %")) }
"""

SAFE_PRELUDE_U8 = """\
#[allow(dead_code)]
#[cold]
#[inline(never)]
fn pruv_elem_oor(v: i64) -> ! { panic!("pruv: element out of range: {} (u8 0..=255)", v) }
#[allow(dead_code)]
#[inline]
fn pruv_get_u8(a: &[u8], i: i64) -> i64 { a[pruv_idx(i, a.len())] as i64 }
#[allow(dead_code)]
#[inline]
fn pruv_set_u8(a: &mut [u8], i: i64, v: i64) { let k = pruv_idx(i, a.len()); if v < 0 || v > 255 { pruv_elem_oor(v) } a[k] = v as u8; }
#[allow(dead_code)]
#[inline]
fn pruv_alloc_u8(n: i64, v: i64) -> ::std::vec::Vec<u8> { if n < 0 || n > (1i64 << 40) { pruv_ovf("alloc length outside 0..2^40") } if v < 0 || v > 255 { pruv_elem_oor(v) } vec![v as u8; n as usize] }
"""

CONTRACT_PRELUDE_U8 = """\
#[allow(dead_code)]
#[inline]
fn pruv_cidx_u8(a: &[u8], i: i128) -> i128 { if i >= 0 && i < (a.len() as i128) { a[i as usize] as i128 } else { pruv_ud("array index out of range") } }
"""


def is_u8_array(t):
    return t is not None and t.name == "array_u8"


# G2 A5: [bool] preludes (emitted only for programs that use [bool])
SAFE_PRELUDE_BOOL = """\
#[allow(dead_code)]
#[inline]
fn pruv_get_bool(a: &[bool], i: i64) -> bool { a[pruv_idx(i, a.len())] }
#[allow(dead_code)]
#[inline]
fn pruv_set_bool(a: &mut [bool], i: i64, v: bool) { let k = pruv_idx(i, a.len()); a[k] = v; }
#[allow(dead_code)]
#[inline]
fn pruv_alloc_bool(n: i64, v: bool) -> ::std::vec::Vec<bool> { if n < 0 || n > (1i64 << 40) { pruv_ovf("alloc length outside 0..2^40") } vec![v; n as usize] }
"""

CONTRACT_PRELUDE_BOOL = """\
#[allow(dead_code)]
#[inline]
fn pruv_cidx_bool(a: &[bool], i: i128) -> bool { if i >= 0 && i < (a.len() as i128) { a[i as usize] } else { pruv_ud("array index out of range") } }
"""


def same_field_path(x, name, fields):
    """LG34 (G-LG32-3): the expression x reads exactly the field path `name.fields` (a FieldAccess chain on the
    local `name`, or `move name.fields`)"""
    if isinstance(x, Move):
        return x.name == name and tuple(x.fields) == tuple(fields)
    path = []
    while isinstance(x, FieldAccess):
        path.append(x.field)
        x = x.base
    return isinstance(x, Var) and x.name == name and tuple(path[::-1]) == tuple(fields)


def is_bool_array(t):
    return t is not None and t.name == "array_bool"


# G2 B2: str preludes (emitted only for programs that use str)
STR_PRELUDE = """\
#[allow(dead_code)]
#[inline]
fn pruv_str_boundary(a: &str, i: i64) -> bool { i >= 0 && (i as u64) <= (a.len() as u64) && a.is_char_boundary(i as usize) }
"""

SAFE_PRELUDE_STR = """\
#[allow(dead_code)]
#[cold]
#[inline(never)]
fn pruv_slice_oob(lo: i64, hi: i64, len: usize) -> ! { panic!("pruv: string slice out of bounds: {}..{} (len {})", lo, hi, len) }
#[allow(dead_code)]
#[cold]
#[inline(never)]
fn pruv_slice_cut(lo: i64, hi: i64, len: usize) -> ! { panic!("pruv: UTF-8 boundary error: {}..{} (len {})", lo, hi, len) }
#[allow(dead_code)]
#[inline]
fn pruv_get_str(a: &str, i: i64) -> i64 { a.as_bytes()[pruv_idx(i, a.len())] as i64 }
#[allow(dead_code)]
#[inline]
fn pruv_substr(a: &str, lo: i64, hi: i64) -> ::std::string::String { if !(lo >= 0 && lo <= hi && (hi as u64) <= (a.len() as u64)) { pruv_slice_oob(lo, hi, a.len()) } if !(a.is_char_boundary(lo as usize) && a.is_char_boundary(hi as usize)) { pruv_slice_cut(lo, hi, a.len()) } ::std::string::String::from(&a[(lo as usize)..(hi as usize)]) }
"""

CONTRACT_PRELUDE_STR = """\
#[allow(dead_code)]
#[inline]
fn pruv_cidx_str(a: &str, i: i128) -> i128 { if i >= 0 && i < (a.len() as i128) { a.as_bytes()[i as usize] as i128 } else { pruv_ud("array index out of range") } }
#[allow(dead_code)]
#[inline]
fn pruv_cboundary(a: &str, i: i128) -> bool { i >= 0 && i <= (a.len() as i128) && a.is_char_boundary(i as usize) }
"""


def is_str(t):
    return t is not None and t.name == "str"


def len_params(f):
    """T0: arrays, str and borrowed String parameters checked against 2^40 at public entries."""
    return [p for p in f.params if p.type is not None
            and (is_array(p.type) or is_str(p.type) or f.host_entry and p.type.name == "string")]


def len_guard(f):
    """T0: the length guard lines of a public entry (SPEC section 7 item 6). The fast path is one branch on the
    non-short-circuit OR (`|`) of one compare per array / str length against 2^40, so it is taken exactly when some
    length is above 2^40; it calls a cold, never-inlined, diverging local fn that takes only the lengths and panics on
    the first parameter above 2^40, in parameter order, with the LLVM ABI wrapper's message (`array length` /
    `str length`). The slow fn must diverge: a returning one leaves the optimizer no length bound after the guard and
    measured +9-18% on the budget M4 default lane (an OR of the lengths, whose false positives such as 2^40 and 1 force
    a returning slow fn). Manifest raw_entries = "crate" leaves the check to the same-crate adapter."""
    ps = len_params(f)
    if not ps:
        return []
    names = [p.name for p in ps]
    checks = " ".join('if (%s as u64) > (1u64 << 40) { panic!("pruv: invalid representation: {} (%s)", %s) }'
                      % (p.name, "str length" if p.type.name in ("str", "string") else "array length", p.name) for p in ps)
    big = " | ".join("((%s.len() as u64) > (1u64 << 40))" % n for n in names)
    return ['#[cold] #[inline(never)] fn pruv_len_slow(%s) -> ! { %s unreachable!() }'
            % (", ".join("%s: usize" % n for n in names), checks),
            'if %s { pruv_len_slow(%s) }' % (big, ", ".join("%s.len()" % n for n in names))]


def str_lit(bs):
    """Rust literal of UTF-8 bytes: printable ASCII except " and \\ as is, everything else \\u{hex}."""
    out = []
    for ch in bs.decode("utf-8"):
        o = ord(ch)
        if 0x20 <= o <= 0x7E and ch != '"' and ch != "\\":
            out.append(ch)
        else:
            out.append("\\u{%x}" % o)
    return '"' + "".join(out) + '"'


SAFE_ARITH = {"+": "pruv_add", "-": "pruv_sub", "*": "pruv_mul", "/": "pruv_div", "%": "pruv_rem"}
CONTRACT_ARITH = {"+": "pruv_cadd", "-": "pruv_csub", "*": "pruv_cmul", "/": "pruv_cdiv", "%": "pruv_crem"}
# P2: bit builtin -> Rust operator, derived from check.BIT_BUILTINS (add entries there). The contract helper
# pruv_c<name> is emitted only when a contract uses the builtin.
BIT_RUST = {n: rust for n, (rust, _llvm) in BIT_BUILTINS.items()}
# P2 batch 3: wrapping arithmetic is an i64 method call (an identifier in BIT_BUILTINS) instead of an operator
WRAP_RUST = tuple(n for n, rust in BIT_RUST.items() if rust.isidentifier())
CONTRACT_WRAP_HELPER = (
    "#[allow(dead_code)]\n"
    "#[inline]\n"
    "fn pruv_c%s(a: i128, b: i128) -> i128 { if a < (i64::MIN as i128) || a > (i64::MAX as i128) || "
    "b < (i64::MIN as i128) || b > (i64::MAX as i128) { pruv_ud(\"argument outside i64 in %s\") } "
    "((a as i64).%s(b as i64)) as i128 }\n")
# P2 batch 4: load_be in a runtime contract (included only when used)
CONTRACT_LOAD_HELPER = (
    "#[allow(dead_code)]\n"
    "#[inline]\n"
    "fn pruv_cload_be(a: &[u8], i: i128, n: usize) -> i128 { if i < 0 || i > (a.len() as i128) - (n as i128) "
    "{ pruv_ud(\"array index out of range\") } let mut w: u64 = 0; "
    "for k in 0..n { w = (w << 8) | (a[(i as usize) + k] as u64); } ((w as i64) as i128) }\n")
CONTRACT_LOAD_LE_HELPER = CONTRACT_LOAD_HELPER.replace("pruv_cload_be", "pruv_cload_le").replace(
    "for k in 0..n", "for k in (0..n).rev()")
# P2 batch 2: shifts need 0 <= k < 64; --no-verify uses these helpers (included only when used, table order)
SHIFT_SAFE = {n: {"<<": "pruv_shl", ">>": "pruv_ushr"}[rust] for n, rust in BIT_RUST.items() if rust in ("<<", ">>")}
SHIFT_OPS = {"<<": "shl", ">>": "shr"}
SHIFT_OP_HELPER = (
    "#[inline]\n"
    "fn pruv_%s_chk(a: i64, k: i64) -> i64 { if k < 0 || k >= 64 "
    "{ pruv_ovf(\"shift count outside 0..64\") } %s }\n")
SHIFT_OP_BODY = {
    "<<": 'let r = a.wrapping_shl(k as u32); if (r >> k) != a { pruv_ovf("overflow in <<") } r',
    ">>": "a >> k",
}
CONTRACT_OPS = {"&": "and", "|": "or", "^": "xor", **SHIFT_OPS}
CONTRACT_OP_HELPER = (
    "#[inline]\n"
    "fn pruv_c%s_op(a: i128, b: i128) -> i128 { "
    "if a < (i64::MIN as i128) || a > (i64::MAX as i128) { pruv_ud(\"argument outside i64 in %s\") } "
    "%s let r = a %s b; %s r }\n")


def contract_op_helper(op):
    if op in SHIFT_OPS:
        check = 'if b < 0 || b >= 64 { pruv_ud("shift count outside 0..64 in %s") }' % op
    else:
        check = ('if b < (i64::MIN as i128) || b > (i64::MAX as i128) '
                 '{ pruv_ud("argument outside i64 in %s") }') % op
    result_check = ('if r < (i64::MIN as i128) || r > (i64::MAX as i128) '
                    '{ pruv_ud("overflow in <<") }') if op == "<<" else ""
    return CONTRACT_OP_HELPER % (CONTRACT_OPS[op], op, check, op, result_check)


SAFE_SHIFT_HELPER = (
    "#[allow(dead_code)]\n"
    "#[inline]\n"
    "fn %s(a: i64, k: i64) -> i64 { if k < 0 || k >= 64 { pruv_ovf(\"shift count outside 0..64\") } "
    "(((a) as u64) %s ((k) as u32)) as i64 }\n")
CONTRACT_SHIFT_HELPER = (
    "#[allow(dead_code)]\n"
    "#[inline]\n"
    "fn pruv_c%s(a: i128, b: i128) -> i128 { if a < (i64::MIN as i128) || a > (i64::MAX as i128) || "
    "b < (i64::MIN as i128) || b > (i64::MAX as i128) { pruv_ud(\"argument outside i64 in %s\") } "
    "if b < 0 || b >= 64 { pruv_ud(\"shift count outside 0..64 in %s\") } "
    "((((a as i64) as u64) %s (b as u32)) as i64) as i128 }\n")
CONTRACT_BIT_HELPER = (
    "#[allow(dead_code)]\n"
    "#[inline]\n"
    "fn pruv_c%s(a: i128, b: i128) -> i128 { if a < (i64::MIN as i128) || a > (i64::MAX as i128) || "
    "b < (i64::MIN as i128) || b > (i64::MAX as i128) { pruv_ud(\"argument outside i64 in %s\") } "
    "((a as i64) %s (b as i64)) as i128 }\n")


# ---------------------------------------------------------------------------
# G6 (docs/G6_SPEC.md section 12): extra attributes and preludes, emitted only for programs that use G6.
# ---------------------------------------------------------------------------
ALLOW_G6 = ALLOW[:-2] + ", unused_assignments, unused_comparisons)]"

_UD_OLD = """\
#[allow(dead_code)]
#[cold]
#[inline(never)]
fn pruv_ud(what: &str) -> ! { panic!("pruv: requires undefined: {}", what) }
"""

# pruv_ud with a boundary context: a foreign helper sets the context (RAII guard), so that an undefined contract
# inside it reports `pruv: foreign contract undefined: NAME: what`; elsewhere the message is unchanged.
_UD_G6 = """\
::std::thread_local! { static PRUV_CTX: ::std::cell::Cell<&'static str> = ::std::cell::Cell::new(""); }
#[allow(dead_code)]
struct PruvCtx;
impl ::std::ops::Drop for PruvCtx { fn drop(&mut self) { PRUV_CTX.with(|c| c.set("")); } }
#[allow(dead_code)]
fn pruv_ctx(name: &'static str) -> PruvCtx { PRUV_CTX.with(|c| c.set(name)); PruvCtx }
#[allow(dead_code)]
#[cold]
#[inline(never)]
fn pruv_ud(what: &str) -> ! {
    let c: &'static str = PRUV_CTX.with(|c| c.get());
    if c.is_empty() { panic!("pruv: requires undefined: {}", what) } else { panic!("pruv: foreign contract undefined: {}: {}", c, what) }
}
"""
assert _UD_OLD in CONTRACT_PRELUDE
CONTRACT_PRELUDE_G6 = CONTRACT_PRELUDE.replace(_UD_OLD, _UD_G6)

FILE_PRELUDE = ("struct pruv_File { file: ::std::option::Option<::std::fs::File>, "
                "ok: bool, readable: bool, writable: bool, buf: ::std::vec::Vec<u8> }\n")

# G10 proc A2 (docs/G10_PROC_SPEC.md section 5): the Process / ProcessArgs runtime, emitted once in front of the Pruv
# functions of a program that uses them. std only. Lines of the form `//@hook NAME code` are test-only fault paths: they are
# dropped unless the compile-time environment variable PRUV_PROC_TEST_HOOK equals NAME (see proc_prelude), so without the
# variable the generated text contains no fault path. The Rust twin (pruv-core codegen.rs PROC_PRELUDE) holds the same text.
PROC_PRELUDE = r"""#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
struct pruv_ProcessArgs { args: ::std::vec::Vec<::std::string::String> }
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
enum PruvProcMsg { Chunk(::std::vec::Vec<u8>), Eof, Err }
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
struct PruvProcStream {
    rx: ::std::sync::mpsc::Receiver<PruvProcMsg>,
    rest: ::std::vec::Vec<u8>,
    off: usize,
    done: bool,
    failed: bool,
    join: ::std::option::Option<::std::thread::JoinHandle<()>>,
    discard: ::std::sync::Arc<::std::sync::atomic::AtomicBool>,
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
struct pruv_Process {
    child: ::std::option::Option<::std::process::Child>,
    stdin: ::std::option::Option<::std::process::ChildStdin>,
    outq: ::std::option::Option<PruvProcStream>,
    errq: ::std::option::Option<PruvProcStream>,
    error: i64,
    stdin_open: bool,
    stdout_eof: bool,
    stderr_eof: bool,
    stdout_buf: ::std::vec::Vec<u8>,
    stderr_buf: ::std::vec::Vec<u8>,
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
impl ::std::ops::Drop for pruv_Process {
    fn drop(&mut self) {
        // defensive only (must-consume is the contract): an unconsumed handle still kills and reaps its direct child
        self.stdin = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
#[cfg(unix)]
fn pruv_proc_sigpipe() {
    extern "C" { fn signal(signum: i32, handler: usize) -> usize; }
    static PRUV_PROC_ONCE: ::std::sync::Once = ::std::sync::Once::new();
    // a write to a closed pipe is the error 4, never a SIGPIPE death of the Pruv process (SIGPIPE = 13, SIG_IGN = 1)
    PRUV_PROC_ONCE.call_once(|| { unsafe { signal(13, 1); } });
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
#[cfg(not(unix))]
fn pruv_proc_sigpipe() {}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
#[cfg(windows)]
fn pruv_proc_batch(program: &str) -> bool {
    // Windows drops trailing dots and spaces from a file name, so x.bat. and x.bat followed by a space name it too
    let l = program.trim_end_matches(|c: char| c == '.' || c == ' ').to_ascii_lowercase();
    l.ends_with(".bat") || l.ends_with(".cmd")
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
#[cfg(not(windows))]
fn pruv_proc_batch(program: &str) -> bool { false }
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
#[cfg(unix)]
fn pruv_proc_status(st: ::std::process::ExitStatus) -> i64 {
    match st.code() {
        Some(c) => c as i64,
        None => match ::std::os::unix::process::ExitStatusExt::signal(&st) { Some(s) => -(s as i64), None => -1 },
    }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
#[cfg(not(unix))]
fn pruv_proc_status(st: ::std::process::ExitStatus) -> i64 {
    match st.code() { Some(c) => (c as u32) as i64, None => -1 }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_failed(code: i64) -> pruv_Process {
    pruv_Process { child: None, stdin: None, outq: None, errq: None, error: code, stdin_open: false, stdout_eof: false, stderr_eof: false, stdout_buf: ::std::vec::Vec::new(), stderr_buf: ::std::vec::Vec::new() }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_fail(p: &mut pruv_Process, code: i64) {
    // the first published error is kept; an update failure closes stdin and empties the public state
    if p.error == 0 { p.error = code; }
    p.stdin = None;
    p.stdin_open = false;
    p.stdout_buf = ::std::vec::Vec::new();
    p.stderr_buf = ::std::vec::Vec::new();
    p.stdout_eof = false;
    p.stderr_eof = false;
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_reader<R: ::std::io::Read>(mut r: R, tx: ::std::sync::mpsc::Sender<PruvProcMsg>, stream: i64, discard: ::std::sync::Arc<::std::sync::atomic::AtomicBool>) {
    let mut buf: ::std::vec::Vec<u8> = vec![0u8; 8192];
    //@hook read1 if stream == 1 { let _ = tx.send(PruvProcMsg::Err); while let Ok(n) = ::std::io::Read::read(&mut r, &mut buf) { if n == 0 { break; } } return; }
    //@hook read2 if stream == 2 { let _ = tx.send(PruvProcMsg::Err); while let Ok(n) = ::std::io::Read::read(&mut r, &mut buf) { if n == 0 { break; } } return; }
    loop {
        match ::std::io::Read::read(&mut r, &mut buf) {
            Ok(0) => { let _ = tx.send(PruvProcMsg::Eof); return; }
            Ok(n) => {
                // discard mode (wait / kill / setup failure): keep reading to EOF but queue nothing more
                if discard.load(::std::sync::atomic::Ordering::SeqCst) { continue; }
                if tx.send(PruvProcMsg::Chunk(buf[..n].to_vec())).is_err() { return; }
            }
            Err(e) => {
                if e.kind() == ::std::io::ErrorKind::Interrupted { continue; }
                let _ = tx.send(PruvProcMsg::Err);
                return;
            }
        }
    }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_collector<R: ::std::io::Read + Send + 'static>(r: R, stream: i64) -> ::std::option::Option<PruvProcStream> {
    let (tx, rx) = ::std::sync::mpsc::channel::<PruvProcMsg>();
    let discard = ::std::sync::Arc::new(::std::sync::atomic::AtomicBool::new(false));
    let flag = discard.clone();
    let join = match ::std::thread::Builder::new().spawn(move || pruv_proc_reader(r, tx, stream, flag)) {
        Ok(h) => h,
        Err(_) => return None,
    };
    Some(PruvProcStream { rx, rest: ::std::vec::Vec::new(), off: 0, done: false, failed: false, join: Some(join), discard })
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_spawn(program: &str, args: &pruv_ProcessArgs, cwd: &str) -> pruv_Process {
    pruv_proc_spawn_in(program, args, cwd, &[], &[])
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_spawn_env(program: &str, args: &pruv_ProcessArgs, cwd: &str, names: &pruv_ProcessArgs, values: &pruv_ProcessArgs) -> pruv_Process {
    pruv_proc_spawn_in(program, args, cwd, &names.args, &values.args)
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_spawn_in(program: &str, args: &pruv_ProcessArgs, cwd: &str, names: &[::std::string::String], values: &[::std::string::String]) -> pruv_Process {
    pruv_proc_sigpipe();
    if program.is_empty() || program.contains('\0') || cwd.contains('\0') || args.args.iter().any(|a| a.contains('\0')) {
        return pruv_proc_failed(1);
    }
    if names.len() != values.len() || names.iter().any(|k| k.is_empty() || k.contains('=') || k.contains('\0')) || values.iter().any(|v| v.contains('\0')) {
        return pruv_proc_failed(1);
    }
    if pruv_proc_batch(program) {
        return pruv_proc_failed(1);
    }
    let mut cmd = ::std::process::Command::new(program);
    cmd.args(args.args.iter());
    for (k, v) in names.iter().zip(values.iter()) {
        cmd.env(k, v);
    }
    if !cwd.is_empty() {
        cmd.current_dir(cwd);
    }
    cmd.stdin(::std::process::Stdio::piped());
    cmd.stdout(::std::process::Stdio::piped());
    cmd.stderr(::std::process::Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return pruv_proc_failed(2),
    };
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let mut outq: ::std::option::Option<PruvProcStream> = None;
    let mut errq: ::std::option::Option<PruvProcStream> = None;
    let mut bad: bool = stdin.is_none() || stdout.is_none() || stderr.is_none();
    //@hook setup bad = true;
    if !bad {
        outq = match stdout { Some(r) => pruv_proc_collector(r, 1), None => None };
        errq = match stderr { Some(r) => pruv_proc_collector(r, 2), None => None };
        bad = outq.is_none() || errq.is_none();
        //@hook setup_late bad = true;
    }
    if bad {
        // setup failed after the spawn: release stdin, kill and reap the direct child, stop and join the started readers
        drop(stdin);
        let _ = child.kill();
        let _ = child.wait();
        if let Some(q) = outq.take() { pruv_proc_discard(&q); let _ = pruv_proc_finish_stream(q); }
        if let Some(q) = errq.take() { pruv_proc_discard(&q); let _ = pruv_proc_finish_stream(q); }
        return pruv_proc_failed(2);
    }
    pruv_Process { child: Some(child), stdin, outq, errq, error: 0, stdin_open: true, stdout_eof: false, stderr_eof: false, stdout_buf: ::std::vec::Vec::new(), stderr_buf: ::std::vec::Vec::new() }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_take(q: &mut PruvProcStream, max: usize) -> ::std::option::Option<::std::vec::Vec<u8>> {
    // at most max bytes of the queue; blocks until bytes, EOF or an error; None = the stream failed
    loop {
        if q.off < q.rest.len() {
            let end: usize = q.off + ::std::cmp::min(q.rest.len() - q.off, max);
            let out: ::std::vec::Vec<u8> = q.rest[q.off..end].to_vec();
            q.off = end;
            if q.off == q.rest.len() {
                q.rest.clear();
                q.off = 0;
            }
            return Some(out);
        }
        if q.done {
            return Some(::std::vec::Vec::new());
        }
        if q.failed {
            return None;
        }
        match q.rx.recv() {
            Ok(PruvProcMsg::Chunk(c)) => { q.rest = c; q.off = 0; }
            Ok(PruvProcMsg::Eof) => { q.done = true; }
            Ok(PruvProcMsg::Err) | Err(_) => { q.failed = true; }
        }
    }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_collect(q: &mut PruvProcStream, out: &mut ::std::vec::Vec<u8>) -> bool {
    // everything not yet delivered, up to EOF; false = the stream failed
    loop {
        if q.off < q.rest.len() {
            out.extend_from_slice(&q.rest[q.off..]);
            q.rest.clear();
            q.off = 0;
        }
        if q.done {
            return true;
        }
        if q.failed {
            return false;
        }
        match q.rx.recv() {
            Ok(PruvProcMsg::Chunk(c)) => { q.rest = c; q.off = 0; }
            Ok(PruvProcMsg::Eof) => { q.done = true; }
            Ok(PruvProcMsg::Err) | Err(_) => { q.failed = true; }
        }
    }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_discard(q: &PruvProcStream) {
    q.discard.store(true, ::std::sync::atomic::Ordering::SeqCst);
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_finish_stream(mut q: PruvProcStream) -> bool {
    // discard the rest of a stream up to EOF, then join its reader thread; false = the stream failed
    let mut ok: bool = true;
    loop {
        if q.done { break; }
        if q.failed { ok = false; break; }
        match q.rx.recv() {
            Ok(PruvProcMsg::Chunk(_)) => {}
            Ok(PruvProcMsg::Eof) => { q.done = true; }
            Ok(PruvProcMsg::Err) | Err(_) => { q.failed = true; }
        }
    }
    if let Some(h) = q.join.take() {
        if h.join().is_err() { ok = false; }
    }
    ok
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_write(mut p: pruv_Process, data: &[u8]) -> pruv_Process {
    if p.error != 0 { return p; }
    let ok: bool = match p.stdin.as_mut() {
        Some(w) => ::std::io::Write::write_all(w, data).is_ok(),
        None => false,
    };
    if !ok { pruv_proc_fail(&mut p, 4); }
    p
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_close(mut p: pruv_Process) -> pruv_Process {
    if p.error != 0 { return p; }
    let mut bad: bool = p.stdin.is_none();
    //@hook close bad = true;
    p.stdin = None;
    p.stdin_open = false;
    if bad { pruv_proc_fail(&mut p, 4); }
    p
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_read(mut p: pruv_Process, stream: i64, max: i64) -> pruv_Process {
    if stream != 1 && stream != 2 { pruv_g6_repr("stream"); }
    if max <= 0 || max > (1i64 << 40) { pruv_g6_repr("max"); }
    if p.error != 0 { return p; }
    let lim: usize = ::std::cmp::min(max as u64, usize::MAX as u64) as usize;
    let got: ::std::option::Option<::std::vec::Vec<u8>> = {
        let q = if stream == 1 { p.outq.as_mut() } else { p.errq.as_mut() };
        match q { Some(q) => pruv_proc_take(q, lim), None => None }
    };
    match got {
        Some(chunk) => {
            let eof: bool = chunk.is_empty();
            if stream == 1 { p.stdout_buf = chunk; p.stdout_eof = eof; } else { p.stderr_buf = chunk; p.stderr_eof = eof; }
        }
        None => { pruv_proc_fail(&mut p, 3); }
    }
    p
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_read_all(mut p: pruv_Process) -> pruv_Process {
    if p.error != 0 { return p; }
    let mut ok: bool = true;
    let mut a: ::std::vec::Vec<u8> = ::std::vec::Vec::new();
    let mut b: ::std::vec::Vec<u8> = ::std::vec::Vec::new();
    // fixed publication order: stdout, then stderr (both are always drained to EOF)
    match p.outq.as_mut() { Some(q) => { if !pruv_proc_collect(q, &mut a) { ok = false; } } None => { ok = false; } }
    match p.errq.as_mut() { Some(q) => { if !pruv_proc_collect(q, &mut b) { ok = false; } } None => { ok = false; } }
    if ok {
        p.stdout_buf = a;
        p.stderr_buf = b;
        p.stdout_eof = true;
        p.stderr_eof = true;
    } else {
        pruv_proc_fail(&mut p, 3);
    }
    p
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_finish(mut p: pruv_Process, kill: bool) -> ::std::result::Result<i64, i64> {
    // wait / kill: release stdin, (kill,) reap the direct child, discard the rest of both outputs and join the readers
    p.stdin = None;
    p.stdin_open = false;
    let mut code: i64 = 0;
    let mut op_err: i64 = 0;
    // both collectors stop queueing before the wait, so output the child writes meanwhile is not held in memory
    if let Some(q) = p.outq.as_ref() { pruv_proc_discard(q); }
    if let Some(q) = p.errq.as_ref() { pruv_proc_discard(q); }
    if let Some(mut c) = p.child.take() {
        let mut kbad: bool = false;
        if kill {
            let exited: bool = match c.try_wait() { Ok(Some(_)) => true, _ => false };
            if !exited {
                if c.kill().is_err() {
                    // the child may have exited between the check and the request: that race is not a failure
                    kbad = match c.try_wait() { Ok(Some(_)) => false, _ => true };
                }
            }
            //@hook kill kbad = true;
        }
        let mut st = c.wait();
        //@hook wait st = Err(::std::io::Error::new(::std::io::ErrorKind::Other, "pruv test wait fault"));
        match st {
            Ok(s) => { code = pruv_proc_status(s); if kbad { op_err = 6; } }
            Err(_) => { op_err = if kbad { 6 } else { 5 }; }
        }
    }
    let mut cbad: bool = false;
    if let Some(q) = p.outq.take() { if !pruv_proc_finish_stream(q) { cbad = true; } }
    if let Some(q) = p.errq.take() { if !pruv_proc_finish_stream(q) { cbad = true; } }
    let err: i64 = if p.error != 0 { p.error } else if op_err != 0 { op_err } else if cbad { 3 } else { 0 };
    if err != 0 { Err(err) } else { Ok(code) }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_buf(p: &pruv_Process, stream: i64) -> &::std::vec::Vec<u8> {
    if stream == 1 { &p.stdout_buf } else if stream == 2 { &p.stderr_buf } else { pruv_g6_repr("stream") }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_wf(p: &pruv_Process) -> bool {
    p.error >= 0 && p.error <= 6 && (p.error == 0 || (!p.stdin_open && p.stdout_buf.is_empty() && p.stderr_buf.is_empty() && !p.stdout_eof && !p.stderr_eof))
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_args_new() -> pruv_ProcessArgs { pruv_ProcessArgs { args: ::std::vec::Vec::new() } }
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_args_push(mut a: pruv_ProcessArgs, arg: &str) -> pruv_ProcessArgs { a.args.push(::std::string::String::from(arg)); a }
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_cproc_buf(p: &pruv_Process, stream: i128) -> &::std::vec::Vec<u8> {
    if stream == 1 { &p.stdout_buf } else if stream == 2 { &p.stderr_buf } else { pruv_ud("process stream outside 1..2") }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_cproc_eof(p: &pruv_Process, stream: i128) -> bool {
    if stream == 1 { p.stdout_eof } else if stream == 2 { p.stderr_eof } else { pruv_ud("process stream outside 1..2") }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_cproc_byte(p: &pruv_Process, stream: i128, i: i128) -> i128 {
    let b = pruv_cproc_buf(p, stream);
    if i >= 0 && i < (b.len() as i128) { b[i as usize] as i128 } else { pruv_ud("process byte index out of range") }
}
"""


def proc_prelude(hook):
    """PROC_PRELUDE with the test-only fault line of `hook` (PRUV_PROC_TEST_HOOK: setup, setup_late, read1, read2, close, wait or
    kill) switched on; every other `//@hook` line is dropped. An empty or unknown hook leaves no fault path at all."""
    out = []
    for line in PROC_PRELUDE.split("\n"):
        t = line.lstrip()
        if t.startswith("//@hook "):
            name, _, code = t[8:].partition(" ")
            if name == hook:
                out.append(line[:len(line) - len(t)] + code)
            continue
        out.append(line)
    return "\n".join(out)


# G10 net A2 (docs/G10_NET_SPEC.md section 5): the Socket / Listener runtime, emitted once in front of the Pruv functions of a
# program that uses them. std::net only; no threads and no queues. `//@hook NAME code` lines are test-only fault paths, dropped
# unless the compile-time environment variable PRUV_NET_TEST_HOOK equals NAME (see net_prelude). The Rust twin (pruv-core
# codegen.rs NET_PRELUDE) holds the same text.
NET_PRELUDE = r"""#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
struct pruv_Socket {
    stream: ::std::option::Option<::std::net::TcpStream>,
    error: i64,
    write_open: bool,
    eof: bool,
    buf: ::std::vec::Vec<u8>,
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
struct pruv_Listener {
    listener: ::std::option::Option<::std::net::TcpListener>,
    error: i64,
    port: i64,
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_failed(code: i64) -> pruv_Socket {
    pruv_Socket { stream: None, error: code, write_open: false, eof: false, buf: ::std::vec::Vec::new() }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_lfailed(code: i64) -> pruv_Listener {
    pruv_Listener { listener: None, error: code, port: 0 }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_fail(s: &mut pruv_Socket, code: i64) {
    // the first published error is kept; an update failure closes the write side and empties the public state
    if s.error == 0 { s.error = code; }
    s.write_open = false;
    s.buf = ::std::vec::Vec::new();
    s.eof = false;
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_bad_host(host: &str) -> bool {
    host.is_empty() || host.contains('\0')
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_timed_out(e: &::std::io::Error) -> bool {
    e.kind() == ::std::io::ErrorKind::WouldBlock || e.kind() == ::std::io::ErrorKind::TimedOut
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_connect(host: &str, port: i64) -> pruv_Socket {
    if port < 1 || port > 65535 { pruv_g6_repr("port"); }
    if pruv_net_bad_host(host) { return pruv_net_failed(1); }
    let addrs = match ::std::net::ToSocketAddrs::to_socket_addrs(&(host, port as u16)) {
        Ok(a) => a,
        Err(_) => return pruv_net_failed(2),
    };
    for a in addrs {
        if let Ok(st) = ::std::net::TcpStream::connect(a) {
            let _ = st.set_nodelay(true);
            return pruv_Socket { stream: Some(st), error: 0, write_open: true, eof: false, buf: ::std::vec::Vec::new() };
        }
    }
    pruv_net_failed(2)
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_listen(host: &str, port: i64) -> pruv_Listener {
    if port < 0 || port > 65535 { pruv_g6_repr("port"); }
    if pruv_net_bad_host(host) { return pruv_net_lfailed(1); }
    let addrs = match ::std::net::ToSocketAddrs::to_socket_addrs(&(host, port as u16)) {
        Ok(a) => a,
        Err(_) => return pruv_net_lfailed(2),
    };
    for a in addrs {
        if let Ok(l) = ::std::net::TcpListener::bind(a) {
            // the port is read back, so port 0 reports the real one
            let p: i64 = match l.local_addr() { Ok(x) => x.port() as i64, Err(_) => 0 };
            if p >= 1 && p <= 65535 {
                return pruv_Listener { listener: Some(l), error: 0, port: p };
            }
        }
    }
    pruv_net_lfailed(2)
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_accept(l: &pruv_Listener) -> pruv_Socket {
    let mut forced: bool = false;
    //@hook accept forced = true;
    if forced { return pruv_net_failed(5); }
    let lis = match l.listener.as_ref() {
        Some(x) => x,
        None => return pruv_net_failed(5),
    };
    loop {
        match lis.accept() {
            Ok((st, _)) => {
                let _ = st.set_nodelay(true);
                return pruv_Socket { stream: Some(st), error: 0, write_open: true, eof: false, buf: ::std::vec::Vec::new() };
            }
            Err(e) => {
                if e.kind() == ::std::io::ErrorKind::Interrupted { continue; }
                return pruv_net_failed(5);
            }
        }
    }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_write(mut s: pruv_Socket, data: &[u8]) -> pruv_Socket {
    if s.error != 0 { return s; }
    let mut forced: bool = false;
    //@hook write forced = true;
    let mut code: i64 = if forced { 4 } else { 0 };
    if code == 0 {
        code = match s.stream.as_mut() {
            Some(st) => match ::std::io::Write::write_all(&mut *st, data) {
                Ok(()) => 0,
                Err(e) => if pruv_net_timed_out(&e) { 6 } else { 4 },
            },
            None => 4,
        };
    }
    if code != 0 { pruv_net_fail(&mut s, code); }
    s
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_shutdown(mut s: pruv_Socket) -> pruv_Socket {
    if s.error != 0 { return s; }
    let mut forced: bool = false;
    //@hook shutdown forced = true;
    let ok: bool = !forced && match s.stream.as_ref() {
        Some(st) => st.shutdown(::std::net::Shutdown::Write).is_ok(),
        None => false,
    };
    s.write_open = false;
    if !ok { pruv_net_fail(&mut s, 4); }
    s
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_set_timeout(mut s: pruv_Socket, ms: i64) -> pruv_Socket {
    if ms < 0 || ms > 86400000 { pruv_g6_repr("timeout"); }
    if s.error != 0 { return s; }
    let t: ::std::option::Option<::std::time::Duration> = if ms == 0 { None } else { Some(::std::time::Duration::from_millis(ms as u64)) };
    let ok: bool = match s.stream.as_ref() {
        Some(st) => st.set_read_timeout(t).is_ok() && st.set_write_timeout(t).is_ok(),
        None => false,
    };
    if !ok { pruv_net_fail(&mut s, 4); }
    s
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_read_chunk(s: &mut pruv_Socket, lim: usize) -> ::std::result::Result<::std::vec::Vec<u8>, i64> {
    // one read call into a buffer of lim bytes (retried on Interrupted); zero bytes read is EOF
    let mut forced: bool = false;
    //@hook read forced = true;
    if forced { return Err(3); }
    let st = match s.stream.as_mut() {
        Some(x) => x,
        None => return Err(3),
    };
    let mut b: ::std::vec::Vec<u8> = vec![0u8; lim];
    loop {
        match ::std::io::Read::read(&mut *st, &mut b[..]) {
            Ok(n) => {
                b.truncate(n);
                return Ok(b);
            }
            Err(e) => {
                if e.kind() == ::std::io::ErrorKind::Interrupted { continue; }
                return Err(if pruv_net_timed_out(&e) { 6 } else { 3 });
            }
        }
    }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_read(mut s: pruv_Socket, max: i64) -> pruv_Socket {
    if max <= 0 || max > (1i64 << 40) { pruv_g6_repr("max"); }
    if s.error != 0 { return s; }
    // a read may return fewer bytes than max, so one call never allocates more than 1 MiB
    let lim: usize = ::std::cmp::min(max as u64, 1u64 << 20) as usize;
    match pruv_net_read_chunk(&mut s, lim) {
        Ok(chunk) => {
            s.eof = chunk.is_empty();
            s.buf = chunk;
        }
        Err(code) => { pruv_net_fail(&mut s, code); }
    }
    s
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_read_all(mut s: pruv_Socket) -> pruv_Socket {
    if s.error != 0 { return s; }
    let mut forced: bool = false;
    //@hook read forced = true;
    let mut code: i64 = if forced { 3 } else { 0 };
    let mut out: ::std::vec::Vec<u8> = ::std::vec::Vec::new();
    if code == 0 {
        match s.stream.as_mut() {
            Some(st) => {
                let mut b: ::std::vec::Vec<u8> = vec![0u8; 65536];
                loop {
                    match ::std::io::Read::read(&mut *st, &mut b[..]) {
                        Ok(0) => break,
                        Ok(n) => {
                            // the buffer cap: more than 2^40 bytes is a read failure
                            if (out.len() as u64) + (n as u64) > (1u64 << 40) { code = 3; break; }
                            out.extend_from_slice(&b[..n]);
                        }
                        Err(e) => {
                            if e.kind() == ::std::io::ErrorKind::Interrupted { continue; }
                            code = if pruv_net_timed_out(&e) { 6 } else { 3 };
                            break;
                        }
                    }
                }
            }
            None => { code = 3; }
        }
    }
    // a failed read publishes none of this call's data
    if code == 0 {
        s.buf = out;
        s.eof = true;
    } else {
        pruv_net_fail(&mut s, code);
    }
    s
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_close(mut s: pruv_Socket) -> ::std::result::Result<i64, i64> {
    // both directions are shut down (errors ignored), then the descriptor is released
    if let Some(st) = s.stream.take() {
        let _ = st.shutdown(::std::net::Shutdown::Both);
        drop(st);
    }
    if s.error != 0 { Err(s.error) } else { Ok(0) }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_lclose(mut l: pruv_Listener) -> ::std::result::Result<i64, i64> {
    drop(l.listener.take());
    if l.error != 0 { Err(l.error) } else { Ok(0) }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_wf(s: &pruv_Socket) -> bool {
    s.error >= 0 && s.error <= 6 && (s.buf.len() as u64) <= (1u64 << 40) && (s.error == 0 || (!s.write_open && s.buf.is_empty() && !s.eof))
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_net_lwf(l: &pruv_Listener) -> bool {
    l.error >= 0 && l.error <= 2 && (if l.error == 0 { l.port >= 1 && l.port <= 65535 } else { l.port == 0 })
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_cnet_byte(s: &pruv_Socket, i: i128) -> i128 {
    if i >= 0 && i < (s.buf.len() as i128) { s.buf[i as usize] as i128 } else { pruv_ud("socket byte index out of range") }
}
"""


def net_prelude(hook):
    """NET_PRELUDE with the test-only fault line of `hook` (PRUV_NET_TEST_HOOK: accept, read, write or shutdown) switched on;
    every other `//@hook` line is dropped. An empty or unknown hook leaves no fault path at all."""
    out = []
    for line in NET_PRELUDE.split("\n"):
        t = line.lstrip()
        if t.startswith("//@hook "):
            name, _, code = t[8:].partition(" ")
            if name == hook:
                out.append(line[:len(line) - len(t)] + code)
            continue
        out.append(line)
    return "\n".join(out)


G6_PRELUDE = """\
#[allow(dead_code)]
#[cold]
#[inline(never)]
fn pruv_g6_req(name: &str, what: &str) -> ! { panic!("pruv: foreign requires violated: {}: {}", name, what) }
#[allow(dead_code)]
#[cold]
#[inline(never)]
fn pruv_g6_ens(name: &str, what: &str) -> ! { panic!("pruv: foreign ensures violated: {}: {}", name, what) }
#[allow(dead_code)]
#[cold]
#[inline(never)]
fn pruv_g6_repr(what: &str) -> ! { panic!("pruv: invalid representation: {}", what) }
#[allow(dead_code)]
#[inline]
fn pruv_g6_charge(budget: &mut i64, n: usize) { let n: i64 = if n > (i64::MAX as usize) { i64::MAX } else { n as i64 }; *budget = budget.saturating_sub(n); if *budget < 0 { pruv_ud("budget") } }
"""


# ---------------------------------------------------------------------------
# G9 A2: runtime contract helpers for `size` and for equality of extended (buffer-free recursive) values. They are emitted
# only when a runtime requires needs them (see Gen.g9_helper_texts) and share the entry's iteration budget.
# ---------------------------------------------------------------------------
G9_TICK = """\
#[allow(dead_code)]
#[inline]
fn pruv_g9_tick(pruv_budget: &mut i64) { *pruv_budget -= 1; if *pruv_budget < 0 { pruv_ud("contract iteration budget exceeded") } }
"""
G9_HELPER_ALLOW = "#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unreachable_patterns)]"


def nominals_in(t):
    """names of the user types a field of type t can contribute to `size` / structural equality: the type itself, or
    the payload types of Option / Result (arrays, str and scalars contribute nothing), in first-occurrence order"""
    if t.owner is None and t.args and t.name in ("option", "result"):
        return list(dict.fromkeys(n for a in t.args for n in nominals_in(a)))
    if is_user_type(t):
        return [t.name]
    return []


def rty(t, param=False, lifetime=""):
    if (base := view_base(t)) is not None:
        return "&" + lifetime + rty(base, True)[1:]
    if t.owner is None and t.args and t.name in ("option", "result"):  # G3: Option<X> / Result<X>, X != i64
        x = rty(t.args[0], lifetime=lifetime)
        if view_payload(t) is not None:
            return "Option<%s>" % x if t.name == "option" else "Result<%s, i64>" % x
        if t.name == "option":
            return "::std::option::Option<%s>" % x
        e = result_error(t)
        result = "::std::result::Result<%s, %s>" % (x, rty(e))
        return "&" + result if param and is_owned_result(t) else result
    if t.owner is None and t.name in ("vec", "hashmap", "string", "file", "process", "process_args", "socket", "listener", "loop"):   # G6: owned; a borrowed parameter is a reference
        if t.name == "vec":
            r = "::std::vec::Vec<%s>" % rty(t.args[0])
        elif t.name == "hashmap":
            r = "::std::collections::HashMap<%s, %s>" % (rty(t.args[0]), rty(t.args[1]))
        elif t.name == "file":
            r = "pruv_File"
        elif t.name == "process":   # G10 proc A2
            r = "pruv_Process"
        elif t.name == "process_args":
            r = "pruv_ProcessArgs"
        elif t.name == "socket":    # G10 net A2
            r = "pruv_Socket"
        elif t.name == "listener":
            r = "pruv_Listener"
        elif t.name == "loop":
            r = "pruv_Loop"
        else:
            r = "::std::string::String"
        return "&" + r if param else r
    simple = {"i64": "i64", "bool": "bool", "unit": "()", "option": "::std::option::Option<i64>",
              "result": "::std::result::Result<i64, i64>"}
    if t.name in simple:
        return simple[t.name]
    if t.name == "array":
        return "&[i64]" if param else "::std::vec::Vec<i64>"
    if t.name == "array_u8":
        return "&[u8]" if param else "::std::vec::Vec<u8>"
    if t.name == "array_bool":  # G2 A5
        return "&[bool]" if param else "::std::vec::Vec<bool>"
    if t.name == "str":  # G2 B2
        return "&str" if param else "::std::string::String"
    if is_user_type(t):
        return t.name
    raise Exception("codegen: unsupported type %r" % (t,))


def cty(t):
    """contract (i128) mirror of a type"""
    if view_base(t) is not None:
        return rty(t)
    if t.owner is None and t.args and t.name in ("option", "result"):  # G3: C(Option<X>) = Option<C(X)>
        x = "()" if t.name == "result" and t.args[0] == UNIT else cty(t.args[0])  # LB8
        if t.name == "option":
            return "::std::option::Option<%s>" % x
        return "::std::result::Result<%s, %s>" % (x, cty(result_error(t)))
    n = t.name
    if n == "i64":
        return "i128"
    if n == "bool":
        return "bool"
    if n == "string":
        return rty(t)
    if n == "option":
        return "::std::option::Option<i128>"
    if n == "result":
        return "::std::result::Result<i128, i128>"
    if is_user_type(t):
        return "pruv_contract__" + n
    raise Exception("codegen: type %r has no contract representation" % (t,))


def conv(t, x):
    """Rust expression turning the executed value `x` (type t) into its contract value"""
    if t.owner is None and t.args and t.name in ("option", "result"):  # G3: recursive, payload first
        inner = "v" if t.name == "result" and t.args[0] == UNIT else conv(t.args[0], "v")  # LB8
        if t.name == "option":
            return x if inner == "v" else "%s.map(|v| %s)" % (x, inner)
        ok = x if inner == "v" else "%s.map(|v| %s)" % (x, inner)
        if len(t.args) == 1:
            return "%s.map_err(|v| v as i128)" % ok
        err = conv(result_error(t), "v")
        return ok if err == "v" else "%s.map_err(|v| %s)" % (ok, err)
    n = t.name
    if n == "i64":
        return "(%s as i128)" % x
    if n in ("bool", "string") or view_base(t) is not None:
        return x
    if n == "option":
        return "%s.map(|v| v as i128)" % x
    if n == "result":
        return "%s.map(|v| v as i128).map_err(|v| v as i128)" % x
    if is_user_type(t):
        return "pruv_to_c__%s(%s)" % (n, x)
    raise Exception("codegen: type %r has no contract representation" % (t,))


def condensed_type_order(decls):
    """G9 A2: declarations (dict name -> decl, first declaration wins) in dependency-first order over the SCC
    condensation of the field-dependency graph. Iterative Tarjan (no recursion over the type graph): roots are visited in
    declaration order, successors in field order (first occurrence), a component is emitted when its root finishes, and
    the members of one component are listed in declaration order. For an acyclic graph this is exactly the old
    depth-first post-order of TypeTable._build_order, so the order of non-recursive programs does not change."""
    names = list(decls)
    index = dict((n, i) for i, n in enumerate(names))
    adj = []
    for n in names:
        d = decls[n]
        fields = d.fields if isinstance(d, StructDecl) else [f for v in d.variants for f in v.fields]
        out = []
        for f in fields:
            for m in dependencies(f.type):
                j = index.get(m)
                if j is not None and j not in out:
                    out.append(j)
        adj.append(out)
    count = len(names)
    num = [-1] * count
    low = [0] * count
    onstack = [False] * count
    stack = []
    comps = []
    counter = 0
    for root in range(count):
        if num[root] != -1:
            continue
        num[root] = low[root] = counter
        counter += 1
        stack.append(root)
        onstack[root] = True
        work = [(root, 0)]
        while work:
            v, pos = work.pop()
            if pos < len(adj[v]):
                work.append((v, pos + 1))
                w = adj[v][pos]
                if num[w] == -1:
                    num[w] = low[w] = counter
                    counter += 1
                    stack.append(w)
                    onstack[w] = True
                    work.append((w, 0))
                elif onstack[w]:
                    low[v] = min(low[v], num[w])
            else:
                if low[v] == num[v]:
                    comp = []
                    while True:
                        w = stack.pop()
                        onstack[w] = False
                        comp.append(w)
                        if w == v:
                            break
                    comps.append(comp)
                if work:
                    parent = work[-1][0]
                    low[parent] = min(low[parent], low[v])
    return [decls[names[i]] for comp in comps for i in sorted(comp)]


def static_str_locals(f, fmap):
    """LB9 SL (R-6g8-3): immutable `let x: str = "lit";` locals whose every read is a borrow (a `str` parameter of a
    catalog or user call, `len`, an index, or the source of `subview` / `substr`) stay `&'static str`, as hand Rust
    writes `let x = "lit";`. A name bound twice anywhere in the function is left alone."""
    lets, binders, bad, hidden = {}, [p.name for p in f.params], set(), []

    def str_param(p):
        return p.type == STR and p.mode is None and not p.take

    def visit(e, ok=False):
        if isinstance(e, (Var, Move)):
            if (not ok or isinstance(e, Move)) and e.name not in hidden:
                bad.add(e.name)
            return
        if isinstance(e, Quant):
            # a quantifier binder shadows a local of the same name (contract expansion may inline one)
            visit(e.lo)
            visit(e.hi)
            hidden.append(e.var)
            for k in [e.body] + list(e.triggers):
                visit(k)
            hidden.pop()
            return
        if isinstance(e, Index):
            visit(e.array, True)
            visit(e.index)
            return
        if isinstance(e, Call):
            n, a = e.name, e.args
            if n == "len" and len(a) == 1 or n in ("subview", "substr") and a:
                flags = [True] + [False] * (len(a) - 1)
            elif n in fmap:
                ps = fmap[n].params
                flags = [i < len(ps) and str_param(ps[i]) for i in range(len(a))]
            elif n in CATALOG:
                ps = CATALOG[n].params
                flags = [i < len(ps) and str_param(ps[i]) for i in range(len(a))]
            else:
                flags = [False] * len(a)
            for x, ok_x in zip(a, flags):
                visit(x, ok_x)
            return
        for k in _kids(e):
            visit(k)

    def block(stmts):
        for st in stmts:
            if isinstance(st, Let):
                binders.append(st.name)
                if not st.mutable and st.type == STR and isinstance(st.value, StrLit) and st.refine is None:
                    lets[st.name] = st
            elif isinstance(st, ForMap):
                binders.extend([st.key, st.val])
            elif isinstance(st, Match):
                for arm in st.arms:
                    binders.extend(arm.binders)
            for name in ("value", "index", "cond", "decreases", "place", "expr", "scrutinee"):
                x = getattr(st, name, None)
                if isinstance(x, Expr):
                    visit(x)
            for name in ("invariants", "calls"):
                for x in getattr(st, name, None) or []:
                    visit(x)
            for name in ("then", "els", "body"):
                block(getattr(st, name, None) or [])
            if isinstance(st, Match):
                for arm in st.arms:
                    block(arm.body)

    block(f.body)
    return {n for n in lets if n not in bad and binders.count(n) == 1}


class CondensedTypeTable(TypeTable):
    """TypeTable whose dependency order is computed on the SCC condensation (G9 programs only)."""
    def _build_order(self):
        self.order = condensed_type_order(self.decls)


def make_type_table(prog):
    return CondensedTypeTable(prog) if getattr(prog, "g9", None) is not None else TypeTable(prog)


def g9_sets(prog):
    """(extended declaration names, boxed field labels) of a G9 program; two empty sets otherwise.
    A boxed label is "Struct.field" or "Enum::Variant.field" (check.py field_label)."""
    meta = getattr(prog, "g9", None)
    if meta is None:
        return set(), set()
    return set(meta["extended"]), set(meta["boxed"])


def emit_public_types(prog, order=None):
    """`pub struct` / `pub enum` for every user type, dependency-first (TypeTable.order).
    G9: an extended type (recursive, or holding a buffer or an extended type) derives Clone, never Copy; a boxed field
    (recursive edge inside a recursive type SCC) is `::std::boxed::Box<Child>`; buffer fields are Vec / String.
    Every other type keeps its exact old text."""
    ext, boxed = g9_sets(prog)
    explicit = bool(getattr(prog, "explicit", False))   # G4 A2: only a source-`pub` type is a `pub` Rust item
    out = []
    types = {d.name: d for d in prog.types}
    for d in (order if order is not None else make_type_table(prog).order):
        pv = "pub " if (not explicit or d.pub) else ""
        out.append("#[allow(dead_code, non_camel_case_types, non_snake_case%s)]" % (", private_interfaces" if explicit else ""))
        if d.name in ext:
            out.append("#[derive(Clone, Debug)]")
        else:
            out.append("#[derive(Clone, Copy, Debug, PartialEq, Eq)]")

        def fty(f, label):
            if f.ref:
                return "&'a " + rty(f.type, True)[1:]
            decl = types.get(f.type.name)
            r = rty(f.type) + ("<'a>" if borrowing_struct(decl) else "")
            return "::std::boxed::Box<%s>" % r if label in boxed else r
        if isinstance(d, StructDecl):
            name = d.name + ("<'a>" if d.borrowing else "")
            if d.fields:
                body = ", ".join("%s%s: %s" % (pv, f.name, fty(f, "%s.%s" % (d.name, f.name))) for f in d.fields)
                out.append("%sstruct %s { %s }" % (pv, name, body))
            else:
                out.append("%sstruct %s {}" % (pv, d.name))
        else:
            vs = []
            for v in d.variants:
                if v.fields:
                    vs.append("%s(%s)" % (v.name, ", ".join(
                        fty(f, "%s::%s.%s" % (d.name, v.name, f.name)) for f in v.fields)))
                else:
                    vs.append(v.name)
            out.append("%senum %s { %s }" % (pv, d.name, ", ".join(vs)))
        out.append("")
    return "\n".join(out) + ("\n" if out else "")


def _exit_bound(cond):
    """the leftmost conjunct of a `while` condition when it reads `i < X` (X a literal or another local): (i, X)"""
    c = cond
    while isinstance(c, Binary) and c.op == "&&":
        c = c.left
    if not (isinstance(c, Binary) and c.op == "<" and isinstance(c.left, Var)):
        return None
    x = c.right
    if isinstance(x, IntLit) or (isinstance(x, Var) and x.name != c.left.name):
        return c.left.name, x
    return None


def _same_bound(a, b):
    if isinstance(a, IntLit) and isinstance(b, IntLit):
        return a.value == b.value
    return isinstance(a, Var) and isinstance(b, Var) and a.name == b.name


def loop_exits(w):
    """Assignments `i = X` in tail position of a `while i < X && ...` body: nothing runs between them and the next
    condition test, whose leftmost conjunct is then false, so the emitted Rust leaves the loop with `break;` right
    away. Hand Rust writes this early exit as `break` (or `iter().all`); the dead re-test kept LLVM from treating the
    branch as a loop exit (LB5b: the key_eq plain scan ran 1.5x slower than the hand's)."""
    out = set()
    b = _exit_bound(w.cond)
    if b is None:
        return out
    i, bound = b

    def tail(stmts):
        if not stmts:
            return
        st = stmts[-1]
        if isinstance(st, Assign) and st.name == i and not st.deref and _same_bound(st.value, bound):
            out.add(id(st))
        elif isinstance(st, If):
            tail(st.then)
            tail(st.els)
    tail(w.body)
    return out


def utf8_checked(st):
    """LG35 (G-DB56-3): the `str_from_utf8(x)` call that is the whole value of the first statement of the `if` st when
    st's condition is an `&&` chain with a conjunct `utf8_valid(x)` and no `inout` argument. The condition has just
    checked those same bytes and nothing ran in between that could change x, so a --no-verify build does not check
    them a second time. None otherwise."""
    if not st.then:
        return None
    f = st.then[0]
    v = f.value if isinstance(f, (Let, Assign, Return)) and not (isinstance(f, Assign) and f.deref) else None
    if not (isinstance(v, Call) and v.name == "str_from_utf8" and len(v.args) == 1 and isinstance(v.args[0], Var)):
        return None
    conj, todo = [], [st.cond]
    while todo:
        c = todo.pop()
        if isinstance(c, Binary) and c.op == "&&":
            todo += [c.left, c.right]
        else:
            conj.append(c)

    def inout(e):
        return isinstance(e, InoutArg) or any(inout(k) for k in expr_kids(e))
    if inout(st.cond):
        return None
    x = v.args[0].name
    if any(isinstance(c, Call) and c.name == "utf8_valid" and len(c.args) == 1 and isinstance(c.args[0], Var)
           and c.args[0].name == x for c in conj):
        return v
    return None


def show(e):
    """readable source-like text of a contract expression (for panic messages)"""
    if isinstance(e, (IntLit, BoolLit)):
        return str(e.value).lower() if isinstance(e, BoolLit) else str(e.value)
    if isinstance(e, Var):
        return e.name
    if isinstance(e, Unary):
        return "%s%s" % (e.op, show(e.operand))
    if isinstance(e, Binary):
        return "(%s %s %s)" % (show(e.left), e.op, show(e.right))
    if isinstance(e, Index):
        return "%s[%s]" % (show(e.array), show(e.index))
    if isinstance(e, Call):
        return "%s(%s)" % (e.name, ", ".join(show(a) for a in e.args))
    if isinstance(e, Ctor):
        return e.name if e.arg is None else "%s(%s)" % (e.name, show(e.arg))
    if isinstance(e, Quant):
        return "%s %s in %s..%s: %s" % (e.kind, e.var, show(e.lo), show(e.hi), show(e.body))
    # G3: a specialized node carries the declaration name as written in `src_name` (its name is the generated stem)
    if isinstance(e, StructLit):
        sn = getattr(e, "src_name", e.name)
        if not e.fields:
            return "%s {}" % sn
        return "%s { %s }" % (sn, ", ".join("%s: %s" % (f.name, show(f.value)) for f in e.fields))
    if isinstance(e, EnumCtor):
        sn = getattr(e, "src_name", e.enum_name)
        if not e.args:
            return "%s::%s" % (sn, e.variant)
        return "%s::%s(%s)" % (sn, e.variant, ", ".join(show(a) for a in e.args))
    if isinstance(e, FieldAccess):
        return "%s.%s" % (show(e.base), e.field)
    if isinstance(e, IsVariant):
        return "is_variant(%s, %s::%s)" % (show(e.value), getattr(e, "src_name", e.enum_name), e.variant)
    if isinstance(e, VariantField):
        return "variant_field(%s, %s::%s, %s)" % (show(e.value), getattr(e, "src_name", e.enum_name), e.variant, e.field)
    if isinstance(e, StrLit):  # G2 B2
        return "<str>"
    return "?"


def match_sorted_requires(e):
    """Name of the array `a` if e is structurally
    `forall i in 0..len(a): forall j in i..len(a): a[i] <= a[j]` (inner start `i` or `i + 1`), else None."""
    def is_len(x, name=None):
        if isinstance(x, Call) and x.name == "len" and len(x.args) == 1 and isinstance(x.args[0], Var):
            return x.args[0].name if name is None else (x.args[0].name == name)
        return None
    if not (isinstance(e, Quant) and e.kind == "forall" and isinstance(e.lo, IntLit) and e.lo.value == 0):
        return None
    a = is_len(e.hi)
    if not a:
        return None
    q = e.body
    if not (isinstance(q, Quant) and q.kind == "forall") or q.var == e.var or a in (e.var, q.var):
        return None
    lo = q.lo
    if isinstance(lo, Binary) and lo.op == "+" and isinstance(lo.right, IntLit) and lo.right.value == 1:
        lo = lo.left
    if not (isinstance(lo, Var) and lo.name == e.var and is_len(q.hi, a)):
        return None
    b = q.body
    if not (isinstance(b, Binary) and b.op == "<=" and isinstance(b.left, Index) and isinstance(b.right, Index)):
        return None
    for ix, v in ((b.left, e.var), (b.right, q.var)):
        if not (isinstance(ix.array, Var) and ix.array.name == a and isinstance(ix.index, Var) and ix.index.name == v):
            return None
    return a


FAST_CMP = ("==", "!=", "<", "<=", ">", ">=")


def fast_forall(e, arr_ok):
    """P3 vectorization: the array reads (name, offset), first occurrence order without repeats, when the forall e can
    be prefiltered by a branch-free loop, else None.  The body may use only comparisons of integer atoms joined by
    !, &&, || and ==>; an atom is an integer literal, the quantified variable, a plain i64 variable, len(a) or a[k + c]
    (k the quantified variable, c a literal of at most 2^31).  None of these can panic once every read is in bounds, so
    the prefilter reads the whole range when a guard before it shows that every read is in bounds and the range fits
    the remaining iteration budget.  arr_ok(var) says whether the backend can read the array variable directly."""
    if e.kind != "forall":
        return None
    reads = []

    def arr(x):
        return isinstance(x, Var) and x.ty is not None and x.ty.owner is None and not x.ty.args and \
            x.ty.name in ("array", "array_u8") and x.name != e.var and arr_ok(x)

    def atom(x):
        if isinstance(x, IntLit):
            return True
        if isinstance(x, Var):
            return x.name == e.var or (x.ty is not None and x.ty.owner is None and x.ty.name == "i64" and not x.ty.args)
        if isinstance(x, Call):
            return x.name == "len" and len(x.args) == 1 and arr(x.args[0])
        if isinstance(x, Index):
            if not arr(x.array):
                return False
            ix = x.index
            if isinstance(ix, Var) and ix.name == e.var:
                c = 0
            elif isinstance(ix, Binary) and ix.op in ("+", "-") and isinstance(ix.left, Var) and \
                    ix.left.name == e.var and isinstance(ix.right, IntLit) and 0 <= ix.right.value <= 1 << 31:
                c = ix.right.value if ix.op == "+" else -ix.right.value
            else:
                return False
            if (x.array.name, c) not in reads:
                reads.append((x.array.name, c))
            return True
        return False

    def pred(x):
        if isinstance(x, BoolLit):
            return True
        if isinstance(x, Unary):
            return x.op == "!" and pred(x.operand)
        if isinstance(x, Binary):
            if x.op in ("&&", "||", "==>"):
                return pred(x.left) and pred(x.right)
            if x.op in FAST_CMP:
                return atom(x.left) and atom(x.right)
        return False

    return reads if pred(e.body) else None


FusePlan = namedtuple("FusePlan", "start arrs n_arr elem idx")


def fuse_map(f):
    """P3-F1 (docs/P3_FUSE_DESIGN.md decision 1): the plan of a fused requires check for an element-wise map, or None.
    Purely syntactic.  f returns [i64], has requires, no cost clause, no inout parameter, no refinement; its maximal
    trailing run of requires f.requires[start:] are all `forall t in 0..len(x): P` over distinct [i64] parameters x that
    fast_forall accepts reading exactly x[t]; its body is exactly
        let c: [i64] = alloc(len(X), v); let i: i64 = 0;
        while i < len(X) { c[i] = E; i = i + 1; }  return c;
    with X one of those arrays and E built from + - *, integer literals and x[i] for x among them."""
    if not f.requires or f.cost is not None or f.ret != ARRAY or f.ret_refine is not None:
        return None
    if any(p.mode == "inout" or p.refine is not None for p in f.params):
        return None
    arrp = {p.name for p in f.params if p.type == ARRAY and p.type.owner is None and not p.type.args}
    start, arrs = len(f.requires), []
    for q in reversed(f.requires):
        if not (isinstance(q, Quant) and q.kind == "forall" and isinstance(q.lo, IntLit) and q.lo.value == 0 and
                isinstance(q.hi, Call) and q.hi.name == "len" and len(q.hi.args) == 1 and isinstance(q.hi.args[0], Var)):
            break
        x = q.hi.args[0].name
        if x not in arrp or fast_forall(q, lambda v: v.name in arrp) != [(x, 0)]:
            break
        start -= 1
        arrs.insert(0, x)
    if start == len(f.requires) or len(set(arrs)) != len(arrs):
        return None
    b = f.body
    if len(b) != 4:
        return None
    s1, s2, s3, s4 = b
    if not (isinstance(s1, Let) and s1.type == ARRAY and s1.refine is None and isinstance(s1.value, Call) and
            s1.value.name == "alloc" and len(s1.value.args) == 2 and isinstance(s1.value.args[1], IntLit)):
        return None
    c = s1.name
    n = s1.value.args[0]
    if not (isinstance(n, Call) and n.name == "len" and len(n.args) == 1 and isinstance(n.args[0], Var)):
        return None
    X = n.args[0].name
    if not (isinstance(s2, Let) and s2.type == I64 and s2.refine is None and isinstance(s2.value, IntLit) and
            s2.value.value == 0):
        return None
    i = s2.name
    if X not in arrs or i == c or c in arrp or i in arrp or c in {p.name for p in f.params} or \
            i in {p.name for p in f.params}:
        return None
    if not (isinstance(s3, While) and isinstance(s3.cond, Binary) and s3.cond.op == "<" and
            isinstance(s3.cond.left, Var) and s3.cond.left.name == i and isinstance(s3.cond.right, Call) and
            s3.cond.right.name == "len" and len(s3.cond.right.args) == 1 and isinstance(s3.cond.right.args[0], Var) and
            s3.cond.right.args[0].name == X and len(s3.body) == 2):
        return None
    st, inc = s3.body
    if not (isinstance(st, IndexAssign) and st.name == c and isinstance(st.index, Var) and st.index.name == i and
            st.elem_refine is None):
        return None
    if not (isinstance(inc, Assign) and inc.name == i and isinstance(inc.value, Binary) and inc.value.op == "+" and
            isinstance(inc.value.left, Var) and inc.value.left.name == i and isinstance(inc.value.right, IntLit) and
            inc.value.right.value == 1):
        return None
    if not (isinstance(s4, Return) and isinstance(s4.value, Var) and s4.value.name == c):
        return None

    def elem_ok(e):
        if isinstance(e, IntLit):
            return True
        if isinstance(e, Binary):
            return e.op in ("+", "-", "*") and elem_ok(e.left) and elem_ok(e.right)
        return isinstance(e, Index) and isinstance(e.array, Var) and e.array.name in arrs and \
            isinstance(e.index, Var) and e.index.name == i
    if not elem_ok(st.value):
        return None
    return FusePlan(start, arrs, X, st.value, i)


def rstr(text):
    """Rust string literal (ASCII-safe)"""
    out = []
    for ch in text:
        if ch in '\\"':
            out.append("\\" + ch)
        elif 32 <= ord(ch) < 127:
            out.append(ch)
        else:
            out.append("\\u{%x}" % ord(ch))
    return '"' + "".join(out) + '"'


class Gen:
    def __init__(s, prog, verified=True, trust_callers=False, budget=1 << 30, foreign_checks="requires", storage=None, stack=None):
        s.storage = storage
        s.stack = stack     # P3-S1 stack plan (None: every array on the heap)
        s.function_name = ""
        s.function_pkg = None
        s.budget = budget
        s.prog = prog
        s.verified = verified
        s.exits = set()    # loop_exits: ids of tail `i = X` assignments followed by `break;`
        s.utf8_ok = None   # LG35: the str_from_utf8 call whose bytes the enclosing `if` checked (utf8_checked)
        s.trust = trust_callers
        s.out = []
        s.cost = False
        s.static_strs = set()
        s.owned_binders = set()   # LB9 (R-6g8-4): struct binders of a consuming match in the current arm
        s.borrow_from = None
        s.cost_used = not verified and any(f.cost is not None for f in prog.functions)
        s.has_req = {f.name for f in prog.functions if f.requires and f.foreign is None}
        from foreign import _par_nodes
        groups = []
        for f in prog.functions:
            _par_nodes(f.body, groups)
        s.par_callees = {c.name for g in groups for c in g.calls}
        # Compile-time PRUV_G7_TEST_HOOK=spawn|panic|overlap is test-only.
        # Fail spawn 2 / panic worker 1 / rendezvous all workers; report each completed join.
        # Without this opt-in, generated output contains no test instrumentation.
        import os
        s.par_hook = os.environ.get("PRUV_G7_TEST_HOOK", "")
        # Compile-time PRUV_PROC_TEST_HOOK=setup|read1|read2|close|wait|kill is test-only (G10 proc A2): it switches on one
        # fault line of the Process runtime. Without it the generated output contains no fault path.
        s.proc_hook = os.environ.get("PRUV_PROC_TEST_HOOK", "")
        # Compile-time PRUV_NET_TEST_HOOK=accept|read|write|shutdown is test-only (G10 net A2): it switches on one fault line
        # of the Socket / Listener runtime. Without it the generated output contains no fault path.
        s.net_hook = os.environ.get("PRUV_NET_TEST_HOOK", "")

        # G4 A2 (explicit visibility mode, docs/G4_SPEC.md 4): only host entries are `pub`. Verified: just the entries
        # that have a requires get a checked wrapper (host boundary; none under trust_callers), every Pruv call goes to
        # the body. Safe mode (--no-verify): EVERY function with a requires has a checked wrapper and every Pruv call goes
        # through it; under trust_callers the entry's pub wrapper skips the checks and an extra private checked wrapper
        # `<entry>__pruv_chk` serves the Pruv callers.
        s.explicit = bool(getattr(prog, "explicit", False))
        s.entries = {f.name for f in prog.functions if f.host_entry}
        # P09: manifest raw_entries ("crate" / "unsafe"): host entries become pub(crate) and the pruv_raw module is
        # appended (raw_module); None keeps today's output byte for byte
        s.raw = getattr(prog, "raw_entries", None) if s.explicit else None
        # T0: only manifest "crate" delegates the 2^40 length check to the same-crate host adapter ("c" keeps it)
        s.raw_crate = s.raw == "crate"
        if s.raw == "c":   # P10: the Rust backend treats "c" exactly as "crate" (no extern "C")
            s.raw = "crate"
        s.proven_entries = False
        s.proven_items = []
        s.raw_items = []    # (name, params text, return text, parameter names) of each host entry's public item, in emission order
        s.checks_needed = s.explicit and any(
            f.requires and f.foreign is None and (not verified or (f.host_entry or f.borrow_from) and not trust_callers)
            for f in prog.functions)
        s.cp = None    # set during requires entry checks: user-type params already converted
        s.bound = []   # quantifier variables in scope (contract evaluation)
        s.tmp = 0
        s.table = make_type_table(prog)
        s.params = set()
        s.take_arrays = set()
        s.take_ext = set()   # P32L: `take` parameters of an extended struct type (owned, borrowed with & in contracts)
        s.inouts = set()     # G5 A2: inout parameters of the function being generated (`&mut [T]` bindings)
        # G5 A2: the inout code paths only run for a program that uses inout (unchecked program: assume it may)
        s.g5 = getattr(prog, "uses_inout", None) is not False
        # G5 A2 (legacy mode): a function with an inout parameter has an always-checked private wrapper, also under
        # trust_callers, so its contract prelude is needed there too
        s.inout_req = s.g5 and (not s.explicit) and any(
            f.requires and f.foreign is None and any(p.mode == "inout" for p in f.params) for f in prog.functions)
        s.vt = {}
        s.borrowed = set()
        # G9 A2 (docs/G9_SPEC.md 6, 12): `ext` = extended declarations (Clone, never Copy), `boxed` = field labels that
        # are Box edges, `refs` = names bound to a Rust reference (extended parameters, match payload views, buffer
        # payload views), `need_size` / `need_eq` = types whose runtime contract helpers are needed (filled while the
        # requires checks are generated, closed over field types by g9_helper_texts). All empty for a non-G9 program.
        s.g9 = getattr(prog, "g9", None) is not None
        s.ext, s.boxed = g9_sets(prog)
        s.rec_vec_elems = set((getattr(prog, "g9", None) or {}).get("rec_vec_elems", ()))
        s.refs = set()
        s.moving = set()     # owned extended locals a return statement may move (see return_movable)
        s.pinned = set()     # names borrowed as the scrutinee of an enclosing match_ext arm
        s.frames = []        # enclosing blocks being emitted: [names declared so far, names named later, loop body]
        s.try_values = {}    # `e?` Ok binder -> `e?` text, for the statement that takes the value (try_match)
        s.need_size = set()
        s.need_eq = set()
        s.need_bits = set()  # P2: band/bor/bxor used in a runtime contract (filled while contracts are generated)
        s.need_shift = set()  # P2 batch 2: shifts used in code under --no-verify (pruv_shl / pruv_ushr helpers)
        s.printed = False  # P13a.10: print lowers to println!, which panics when stdout fails (raw_unwind_free)
        s.need_load_le = False
        s.need_cidx_u8 = False
        s.has_cidx_u8 = False
        s.need_load_be = False  # P2 batch 4: load_be used in a runtime contract (pruv_cload_be helper)
        # T0 (TIDY_SPEC 4.8 item 4): `lg` = functions without a requires whose public Rust item takes an array or str
        # (legacy: every non-private function; explicit mode: host entries). Like a function with a requires, such a
        # function has a private body f__pruv_body and a public wrapper f that checks every array / str length against
        # 2^40 (SPEC section 7 item 6) and is not waived by trust_callers; verified Pruv callers call the body, so the
        # check runs once at the host boundary and never on an internal call (TIDY_SPEC 4.3).
        # Manifest raw_entries = "crate" has no length check (the same-crate adapter owns it), so no extra split.
        s.lg = set() if s.raw_crate else {f.name for f in prog.functions
                if f.foreign is None and not f.requires and len_params(f)
                and (f.host_entry if s.explicit else not s.private(f))}
        s.struct_inouts = {f.name for f in prog.functions
                           if any(p.mode == "inout" and not is_array(p.type) for p in f.params)}
        s.body_named = s.has_req | s.lg
        if verified and not s.explicit:
            s.body_named -= s.struct_inouts - {f.name for f in prog.functions
                                              if any(p.mode == "inout" and is_array(p.type) for p in f.params)}
        # G6 (section 12): only a program that uses G6 gets any of the following
        from foreign import first_g6_use, first_proc_use, first_net_use, first_async_use, uses_files
        s.ev = first_async_use(prog) is not None
        s.async_hook = os.environ.get("PRUV_ASYNC_TEST_HOOK", "")
        s.g10 = uses_files(prog)
        s.proc = first_proc_use(prog) is not None   # G10 proc A2: the Process / ProcessArgs runtime
        s.net = first_net_use(prog) is not None     # G10 net A2: the Socket / Listener runtime
        s.g6 = first_g6_use(prog) is not None
        s.fmode = foreign_checks
        s.allow = ALLOW_G6 if s.g6 else ALLOW
        s.fmap = {}
        s.array_resizes = array_resize_params(prog)
        for f in prog.functions:
            s.fmap.setdefault(f.name, f)
        s.fg = None
        if s.g6:
            from foreign_codegen import ForeignGen
            s.fg = ForeignGen(s)
        # KD-M6-3 (docs/SMALL_RESULT_SPEC.md): small array results on the stack, verified mode only
        s.small, s.small_sites = {}, {}
        s.small_o = None      # the local `o` while a __pruv_small variant is emitted
        s.small_call = None   # the call being emitted as a rewritten call site, and the name of its binding
        s.small_buf = None
        s.small_elem = "i64"   # buffer element of the __pruv_small variant being emitted
        if verified and not trust_callers and not s.cost_used:
            from small import plan_small
            s.small, s.small_sites = plan_small(prog, s.fmap, s.layout, s.stack, s.array_resizes)
        s.fp = {}    # P18d: {function name: (K, o)} of the by-value fixed-prefix twins (--proven-entries, see plan_fp)

    def plan_fp(s):
        """P18d: plan the by-value `__pruv_fp` twins; runs after proven_entries is set. The adapter copies the prefix
        back only when the core returns, so a panic after a write would lose that write: plan only in a verified build
        (no overflow or index panic) without G10 runtime or par groups, and never for a function that can reach a
        foreign function, a G6 catalog operation or print (fixed_prefix.may_unwind)."""
        from fixed_prefix import plan_fp, may_unwind
        if not (s.verified and not s.g10 and not s.proc and not s.net and not s.ev and not s.par_callees):
            return
        cands = []
        for f in s.prog.functions:
            if (not s.g5 or f.tparams or f.foreign is not None or f.name in s.entries or f.name in s.body_named
                    or f.name in s.small or not f.params or s.fmap.get(f.name) is not f
                    or (not s.verified and f.cost is not None)):
                continue
            p = f.params[-1]
            if p.mode == "inout" and p.type == ARRAY and (f.name, len(f.params) - 1) not in s.array_resizes:
                cands.append(f)
        s.fp = plan_fp(cands, s.fmap, may_unwind(s.prog, s.fmap))

    def fp_items(s, f, ps, ret, param_types, fp):
        """P18d: the adapter `f` (pinned always) and the by-value twin `f__pruv_fp`; the body follows as `f__pruv_fpb`"""
        k, o = fp
        others = ["%s: %s" % (p.name, ty) for p, ty in zip(f.params[:-1], param_types[:-1])]
        args = [p.name for p in f.params[:-1]]
        rt = "()" if f.ret == UNIT else ret[4:]
        s.emit(0, s.allow)
        s.emit(0, PIN_ALWAYS)
        s.emit(0, "fn %s(%s)%s {" % (f.name, ps, ret))
        s.emit(1, "let pruv_fn: usize = if %s.len() < %d { %s.len() } else { %d };" % (o, k, o, k))
        s.emit(1, "let mut pruv_fa = [0i64; %d];" % k)
        s.emit(1, "pruv_fa[..pruv_fn].copy_from_slice(&%s[..pruv_fn]);" % o)
        if k == 1:
            # one word: (RT, i64) is a scalar pair, returned in registers; (RT, [i64; 1]) is returned through memory
            s.emit(1, "let (pruv_fr, pruv_f0) = %s__pruv_fp(%s);" % (
                f.name, ", ".join(args + ["pruv_fa[0]", "pruv_fn"])))
            s.emit(1, "pruv_fa[0] = pruv_f0;")
        else:
            s.emit(1, "let (pruv_fr, pruv_fa) = %s__pruv_fp(%s);" % (
                f.name, ", ".join(args + ["pruv_fa", "pruv_fn"])))
        s.emit(1, "%s[..pruv_fn].copy_from_slice(&pruv_fa[..pruv_fn]);" % o)
        s.emit(1, "pruv_fr")
        s.emit(0, "}")
        s.emit(0, "")
        s.emit(0, s.allow)
        s.emit_inline(f)
        if k == 1:
            s.emit(0, "fn %s__pruv_fp(%s) -> (%s, i64) {" % (
                f.name, ", ".join(others + ["pruv_f0: i64", "pruv_fn: usize"]), rt))
            s.emit(1, "let mut pruv_fa = [pruv_f0];")
        else:
            s.emit(0, "fn %s__pruv_fp(%s) -> (%s, [i64; %d]) {" % (
                f.name, ", ".join(others + ["mut pruv_fa: [i64; %d]" % k, "pruv_fn: usize"]), rt, k))
        s.emit(1, "let pruv_fr = %s__pruv_fpb(%s);" % (f.name, ", ".join(args + ["&mut pruv_fa[..pruv_fn]"])))
        s.emit(1, "(pruv_fr, pruv_fa[0])" if k == 1 else "(pruv_fr, pruv_fa)")
        s.emit(0, "}")
        s.emit(0, "")

    def emit_inline(s, f):
        if f.attribute == "cold":
            s.emit(0, "#[cold]")
            s.emit(0, "#[inline(never)]")
        else:
            s.emit(0, {"inline": PIN_INLINE, "inline(always)": PIN_ALWAYS}.get(f.attribute, "#[inline]"))

    def emit(s, ind, line):
        s.out.append("    " * ind + line)

    def field_uses(s, name):
        """G9: some struct field / enum payload has the internal type `name` (array_u8, array_bool, str). The G2
        helpers are emitted by usage; uses_array_type / uses_str look at parameters, results and lets only."""
        for d in s.prog.types:
            fields = d.fields if isinstance(d, StructDecl) else [f for v in d.variants for f in v.fields]
            if any(f.type.name == name for f in fields):
                return True
        return False

    def prelude(s):
        if s.g10:
            s.out.append(FILE_PRELUDE)
        if s.proc or s.ev:
            s.out.append(async_proc_prelude(s.proc_hook,s.async_hook) if s.ev else proc_prelude(s.proc_hook))
        if s.net or s.ev:
            s.out.append(net_prelude(s.net_hook))
        if s.ev:
            s.out.append(async_prelude(s.async_hook))
        types = emit_public_types(s.prog, s.table.order if s.g9 else None)
        if types:
            s.out.append(types)
        u8_used = uses_array_type(s.prog, "array_u8") or s.g10
        bool_used = uses_array_type(s.prog, "array_bool")  # G2 A5
        str_used = uses_str(s.prog, cost=not s.verified)  # G2 B2; S5: an erased cost bound emits nothing
        if s.g9:
            u8_used = u8_used or s.field_uses("array_u8")
            bool_used = bool_used or s.field_uses("array_bool")
            str_used = str_used or s.field_uses("str") or s.field_uses("string")
        if s.g6 and not str_used:   # G6: a String parameter can be indexed in a runtime requires (pruv_cidx_str)
            str_used = uses_array_type(s.prog, "string")
        if str_used:
            s.out.append(STR_PRELUDE)
        if not s.verified:
            s.out.append(SAFE_PRELUDE)
            if u8_used:
                s.out.append(SAFE_PRELUDE_U8)
            if bool_used:
                s.out.append(SAFE_PRELUDE_BOOL)
            if str_used:
                s.out.append(SAFE_PRELUDE_STR)
        if (s.checks_needed if s.explicit else (s.has_req and (not s.trust or s.inout_req or (s.par_callees and not s.verified)))) or s.g6 or s.cost_used:
            s.out.append(CONTRACT_PRELUDE_G6 if s.g6 else CONTRACT_PRELUDE)
            if u8_used:
                s.out.append(CONTRACT_PRELUDE_U8)
                s.has_cidx_u8 = True
            if bool_used:
                s.out.append(CONTRACT_PRELUDE_BOOL)
            if str_used:
                s.out.append(CONTRACT_PRELUDE_STR)
            ctypes = s.contract_types()
            if ctypes:
                s.out.append(ctypes)
            if s.g6:
                s.out.append(G6_PRELUDE)

    def contract_types(s):
        """pruv_contract__Name mirror types and pruv_to_c__Name converters (only for requires evaluation)"""
        attrs = "#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables)]"
        out = []
        for d in s.table.order:
            if d.name in s.ext:
                continue    # G9: extended values have no i128 mirror; contracts read them through references (cx)
            cn = "pruv_contract__" + d.name
            out.append(attrs)
            out.append("#[derive(Clone, Copy, PartialEq, Eq)]")
            if isinstance(d, StructDecl):
                out.append("struct %s { %s }" % (cn, ", ".join("%s: %s" % (f.name, cty(f.type)) for f in d.fields)))
                out.append(attrs)
                out.append("#[inline]")
                init = ", ".join("%s: %s" % (f.name, conv(f.type, "v." + f.name)) for f in d.fields)
                out.append("fn pruv_to_c__%s(v: %s) -> %s { %s { %s } }" % (d.name, d.name, cn, cn, init))
            else:
                vs, arms = [], []
                for v in d.variants:
                    if v.fields:
                        vs.append("%s(%s)" % (v.name, ", ".join(cty(f.type) for f in v.fields)))
                        names = ["pruv_a%d" % i for i in range(len(v.fields))]
                        args = ", ".join(conv(f.type, n) for f, n in zip(v.fields, names))
                        arms.append("%s::%s(%s) => %s::%s(%s)," % (d.name, v.name, ", ".join(names), cn, v.name, args))
                    else:
                        vs.append(v.name)
                        arms.append("%s::%s => %s::%s," % (d.name, v.name, cn, v.name))
                out.append("enum %s { %s }" % (cn, ", ".join(vs)))
                out.append(attrs)
                out.append("#[inline]")
                out.append("fn pruv_to_c__%s(v: %s) -> %s { match v { %s } }" % (d.name, d.name, cn, " ".join(arms)))
        return "\n".join(out) + ("\n" if out else "")

    def is_ext(s, t):
        """t is an extended user type (G9): recursive, or holding a buffer or an extended type"""
        return bool(s.ext) and t is not None and t.owner is None and not t.args and t.name in s.ext

    def layout(s, node):
        width = s.storage.layout_of(node) if s.storage else "i64"
        if width == "i64" and isinstance(node, Var) and s.storage and id(node) not in s.storage.objects:
            return s.storage.parameter(s.function_name, node.name)
        return width

    def rt(s, t, param=False, site=None):
        """rty, except that an extended parameter is a shared reference `&Name` (read view)"""
        width = s.layout(site)
        if t == ARRAY and width != "i64":
            return "&[%s]" % width if param else "::std::vec::Vec<%s>" % width
        if param and s.is_ext(t):
            return "&" + t.name
        return rty(t, param)

    def private(s, f):
        """G3 specializations, G6 collection-bearing functions and (G9) functions with an extended type in their
        signature are private Rust items (no raw ABI); this includes their checked entry wrappers"""
        return (getattr(f, "private", False) or any(is_owned_value(p.type) for p in f.params) or is_owned_value(f.ret)
                or any(s.is_ext(p.type) for p in f.params) or s.is_ext(f.ret) or view_payload(f.ret) is not None
                or (s.g5 and any(p.mode == "inout" for p in f.params)))   # G5: no public API for a mutable borrow

    def fn(s, f):
        s.function_name = f.name
        s.borrow_from = f.borrow_from
        name = f.name
        s.function_pkg = getattr(f, "pkg", None)   # G4b-2: package of the function being emitted (trust report)
        if f.foreign is not None:
            return   # G6: a trusted rust fn has no body; its boundary helper is emitted on first use
        s.params = {p.name for p in f.params}
        s.take_arrays = {p.name for p in f.params if p.take and is_array(p.type)}
        s.take_ext = {p.name for p in f.params if p.take and s.is_ext(p.type)}
        if s.g5:
            s.inouts = {p.name for p in f.params if p.mode == "inout"}
        s.resizable = {p.name for i, p in enumerate(f.params) if (f.name, i) in s.array_resizes}
        s.resize_args = array_resize_args(f, s.array_resizes)
        s.vt = {p.name: p.type for p in f.params}
        s.borrowed = {p.name for p in f.params if is_owned_value(p.type) and not p.take}
        s.refs = {p.name for p in f.params if s.is_ext(p.type) and not p.take}
        param_types = [
            ("&mut str" if p.type == STR else
             "&mut " + (rty(p.type, True)[1:] if is_array(p.type) and p.name not in s.resizable else rty(p.type))
             # R-6g9-6: an inout borrowing struct is `&mut Units<'_>`, its own lifetime left free.
             + ("<'_>" if borrowing_struct(s.table.decls.get(p.type.name)) else ""))
            if p.mode == "inout" else s.rt(p.type, not p.take, p) for p in f.params]
        lifetime = "<'a>" if f.borrow_from else ""
        if f.borrow_from:
            field_sources = {entry.split(".")[0] for entry in f.borrow_from if "." in entry}
            for i, (p, ty) in enumerate(zip(f.params, param_types)):
                borrowing = borrowing_struct(s.table.decls.get(p.type.name))
                if p.name in field_sources:
                    param_types[i] = "&'a " + ty[1:] + ("<'_>" if borrowing else "")
                elif p.name in f.borrow_from:
                    param_types[i] = ty + "<'a>" if borrowing else "&'a " + ty[1:]
        s.ref_result = f.ref_result
        s.borrowed_roots = getattr(f, "borrowed_roots", ())
        from foreign_shim import _walk
        moved = set()   # LG9 (G-LG8-3): a take record parameter a consuming field move empties is a `mut` binding

        def field_move(node):
            if isinstance(node, Move) and node.fields:
                moved.add(node.name)
        if any(p.take and is_user_type(p.type) for p in f.params):
            _walk(f.body, field_move)
        ps = ", ".join("%s%s: %s" % ("mut " if p.take and is_user_type(p.type) and p.name in moved else "", p.name, ty)
                       for p, ty in zip(f.params, param_types))
        ret = "" if f.ret == UNIT else " -> " + s.rt(f.ret, site=f)
        if f.ref_result:
            ret = " -> &'a " + rty(f.ret, True)[1:]
        elif view_payload(f.ret) is not None:
            ret = " -> " + rty(f.ret, lifetime="'a ")
        elif f.borrow_from:
            ret += lifetime
        body_name = name + "__pruv_body" if f.name in s.body_named else name
        if s.explicit:
            vis = s.pub_vis() if (f.host_entry and f.name not in s.body_named) else ""
            if vis and s.raw:
                s.raw_items.append((f.name, ps, ret, [p.name for p in f.params]))
        else:
            vis = "" if (f.name in s.body_named or s.private(f)) else "pub "   # G3: specializations are private
        fp = s.fp.get(f.name)
        if fp is not None:     # P18d: adapter + by-value twin first, then the unchanged body under its new name
            s.fp_items(f, ps, ret, param_types, fp)
            body_name, vis = f.name + "__pruv_fpb", ""
        s.emit(0, s.allow)
        # Inline hint on every function body: without it rustc keeps the generated call chains as real calls.
        # inline_policy raises it to #[inline(always)] where the measured policy says so.
        if fp is not None:
            s.emit(0, PIN_ALWAYS)
        else:
            s.emit_inline(f)
        s.emit(0, "%sfn %s%s(%s)%s {" % (vis, body_name, lifetime, ps, ret))
        s.cost = not s.verified and f.cost is not None
        s.static_strs = static_str_locals(f, s.fmap)   # R-6g9-3: also metered, the checker charges them no alloc
        if s.cost:
            args = ["&" + p.name if p.name in s.take_arrays or p.name in s.take_ext
                    or p.take and (p.type.name == "string" or is_owned_result(p.type))
                    else p.name for p in f.params]
            s.emit(1, "let pruv_cost: i128 = %s__pruv_cost(%s);" % (f.name, ", ".join(args)))
            s.emit(1, "let mut pruv_spent: u128 = 0;")
        s.block(f.body, 1)
        if s.cost and f.ret == UNIT:
            s.emit(1, s.cost_check())
        s.emit(0, "}")
        s.emit(0, "")
        if f.name in s.small:
            s.small_fn(f, ps)
        if s.cost:
            take_ext, s.take_ext = s.take_ext, set()   # the cost helper takes every extended parameter as `&Name`
            # Use the requires evaluator on parameter views, including immutable views of inout.
            cps = ", ".join("%s: %s" % (p.name, "&" + rty(p.type)
                                        if p.mode == "inout" and not is_array(p.type) else s.rt(p.type, True))
                            for p in f.params)
            s.emit(0, s.allow)
            s.emit(0, "fn %s__pruv_cost(%s) -> i128 {" % (f.name, cps))
            # The bound is evaluated only inside the callee's requires (spec 5: requires before contract_domain).
            conversions, checks = s.req_checks(f) if f.requires and not s.trust else ([], [])
            for line in conversions:
                s.emit(1, line)
            s.emit(1, "let mut pruv_budget: i64 = %s;" % format(s.budget, "_"))
            for line in checks:
                s.emit(1, line)
            s.emit(1, s.c(f.cost))
            s.emit(0, "}")
            s.emit(0, "")
            s.take_ext = take_ext
        if f.requires and f.name in s.body_named:
            if not s.explicit:
                # G5: a wrapper of an inout function always checks (trust_callers does not waive it)
                s.entry(f, ps, param_types, ret, body_name, checked=True if s.inouts else None)
                if not s.verified and s.trust and not s.inouts and f.name in s.par_callees:
                    s.entry(f, ps, param_types, ret, body_name, f.name + "__pruv_par_chk", False, True)
            elif s.verified:
                if f.host_entry or f.borrow_from:
                    s.entry(f, ps, param_types, ret, body_name, f.name, f.host_entry, not s.trust)
            elif f.host_entry:
                s.entry(f, ps, param_types, ret, body_name, f.name, True, not s.trust)
                if s.trust:          # Pruv callers of the entry keep the safe-mode check
                    s.entry(f, ps, param_types, ret, body_name, f.name + "__pruv_chk", False, True)
            else:
                s.entry(f, ps, param_types, ret, body_name, f.name, False, True)
        elif f.name in s.lg:
            s.entry(f, ps, param_types, ret, body_name, f.name, True, False)   # T0: only the length guard

    def pub_vis(s):
        """P09: the visibility of a host entry's public item"""
        return "pub(crate) " if s.raw else "pub "

    def small_fn(s, f, ps):
        """KD-M6-3: the variant of a small function that writes its result into the caller's stack buffer"""
        k, o, elem = s.small[f.name]
        s.emit(0, s.allow)
        s.emit(0, "fn %s__pruv_small(pruv_sb: &mut [%s; %d]%s) -> usize {" % (f.name, elem, k, ", " + ps if ps else ""))
        s.small_elem = elem
        saved = s.inouts
        s.inouts = set(saved) | {o}
        s.small_o = o
        s.block(f.body, 1)
        s.small_o = None
        s.inouts = saved
        s.emit(0, "}")
        s.emit(0, "")

    @staticmethod
    def cost_add(units):
        return ('pruv_spent = pruv_spent.checked_add(%s).unwrap_or_else(|| panic!("pruv: cost exceeded"));' % units)

    @staticmethod
    def cost_check():
        return 'if pruv_cost < 0 || pruv_spent > pruv_cost as u128 { panic!("pruv: cost exceeded"); }'

    def cost_call(s, name, args):
        # A negative modular bound cannot be met by a nonnegative callee counter.
        args = ["&" + arg if p.take and (is_array(p.type) or p.type.name == "string" or is_owned_result(p.type)
                                       or s.is_ext(p.type)) else arg
                for p, arg in zip(s.fmap[name].params, args)]
        return ('let pruv_units = %s__pruv_cost(%s); '
                'if pruv_units < 0 { panic!("pruv: cost exceeded"); } %s' %
                (name, ", ".join(args), s.cost_add('(pruv_units as u128).checked_add(1).unwrap_or_else(|| panic!("pruv: cost exceeded"))')))

    def callee(s, n):
        """explicit mode: the Rust name a Pruv call of the function n invokes (see Gen.__init__)"""
        if n not in s.has_req:
            return n + "__pruv_body" if (s.verified and n in s.lg) else n   # T0: past the entry's length guard
        if s.verified:
            return n + "__pruv_body"     # the caller proved requires
        return n + "__pruv_chk" if (s.trust and n in s.entries) else n

    def req_checks(s, f):
        """(parameter conversion lines, requires check lines) of f's runtime requires"""
        s.cp = {}   # user-type parameter -> its type, converted once below
        checks = []
        try:
            for n, r in enumerate(f.requires, 1):
                msg = "pruv: requires violated: %s: %s" % (getattr(f, "disp", f.name), show(r))
                code = s.c(r)
                checks.append("if !(%s) { panic!(\"{}\", %s); }" % (code, rstr(msg)))
            used = dict(s.cp)
        finally:
            s.cp = None
        conversions = ["let pruv_cp_%s: %s = %s;" % (p.name, cty(used[p.name]), conv(used[p.name], ("*" + p.name) if p.mode == "inout" and not is_array(p.type) else p.name))
                       for p in f.params if p.name in used]
        return conversions, checks

    def entry(s, f, ps, param_types, ret, body_name, name=None, pub=None, checked=None):
        """the checked wrapper of f. Legacy: `pub` unless private, checks unless trust_callers. Explicit mode passes the
        wrapper name, its visibility and whether it checks."""
        name = f.name if name is None else name
        pub = (not s.private(f)) if pub is None else pub
        checked = (not s.trust) if checked is None else checked
        vis = s.pub_vis() if pub else ""
        header = "%sfn %s%s(%s)%s {" % (vis, name, "<'a>" if f.borrow_from else "", ps, ret)
        s.emit(0, s.allow)
        s.emit_inline(f)
        s.emit(0, header)
        if pub and s.explicit and s.raw:
            s.raw_items.append((name, ps, ret, [p.name for p in f.params]))
        # T0 also applies under trust_callers. The same-crate adapter owns it in raw_entries = "crate".
        guard = len_guard(f) if pub and not s.raw_crate else []
        for line in guard:
            s.emit(1, line)
        body_start = len(s.out)
        if checked:
            conversions, checks = s.req_checks(f)
            for line in conversions:
                s.emit(1, line)
            if checks:
                s.emit(1, "let mut pruv_budget: i64 = %s;" % format(s.budget, "_"))
            plan = s.fuse_plan(f)
            if plan is None:
                for line in checks:
                    s.emit(1, line)
            else:
                # P3-F1: prefix requires, fused path, then the unchanged trailing requires (the slow path)
                for line in checks[:plan.start]:
                    s.emit(1, line)
                s.emit(1, s.fuse_text(f, plan))
                for line in checks[plan.start:]:
                    s.emit(1, line)
        if s.inouts:
            s.emit(1, "%s(%s)" % (body_name, ", ".join(("&mut *" + p.name) if p.mode == "inout" and is_array(p.type) else p.name
                                                        for p in f.params)))
        else:
            s.emit(1, "%s(%s)" % (body_name, ", ".join(p.name for p in f.params)))
        if s.proven_entries and guard:
            twin = proven_entry(f, param_types, vis, name, ret)
            if twin is not None:
                signature, rebinds, has_str, val = twin
                s.proven_items.append((header, s.allow, signature, rebinds + s.out[body_start:], has_str, val))
        s.emit(0, "}")
        s.emit(0, "")

    def fuse_plan(s, f):
        """P3-F1: fuse_map(f) of a verified build whose result and parameter arrays are plain i64 heap arrays, else None"""
        if not s.verified:
            return None
        plan = fuse_map(f)
        if plan is None:
            return None
        if s.stack is not None and s.stack.contains(f.body[0]):
            return None
        if s.layout(f) != "i64" or s.layout(f.body[0].value) != "i64":
            return None
        if s.storage and any(s.storage.parameter(f.name, x) != "i64" for x in plan.arrs):
            return None
        return plan

    def cfuse(s, e, var, plan):
        """P3-F1: the branch-free forall body of a fused path (like cfast); x[t] reads pruv_fxK, t (var) is pruv_fj"""
        if isinstance(e, Unary):
            return "(!%s)" % s.cfuse(e.operand, var, plan)
        if isinstance(e, Binary) and e.op in ("&&", "||", "==>"):
            l, r = s.cfuse(e.left, var, plan), s.cfuse(e.right, var, plan)
            if e.op == "==>":
                return "((!%s) | %s)" % (l, r)
            return "(%s %s %s)" % (l, "&" if e.op == "&&" else "|", r)
        if isinstance(e, Binary):
            return "(%s %s %s)" % (s.cfuse(e.left, var, plan), e.op, s.cfuse(e.right, var, plan))
        if isinstance(e, Index):
            return "(pruv_fx%d as i128)" % plan.arrs.index(e.array.name)
        if isinstance(e, Var) and e.name == var:
            return "(pruv_fj as i128)"
        return s.c(e)

    def fuse_elem(s, e, plan):
        """P3-F1: the map element expression with wrapping i64 arithmetic"""
        if isinstance(e, IntLit):
            return "(%di64)" % e.value if e.value < 0 else "%di64" % e.value
        if isinstance(e, Index):
            return "pruv_fx%d" % plan.arrs.index(e.array.name)
        name = {"+": "wrapping_add", "-": "wrapping_sub", "*": "wrapping_mul"}[e.op]
        return "(%s).%s(%s)" % (s.fuse_elem(e.left, plan), name, s.fuse_elem(e.right, plan))

    def fuse_text(s, f, plan):
        """P3-F1: the one-line fused path (docs/P3_FUSE_DESIGN.md decision 4)"""
        preds = [s.cfuse(q.body, q.var, plan) for q in f.requires[plan.start:]]
        guards = ["%s.len() == pruv_fn" % x for x in plan.arrs if x != plan.n_arr]
        guards.append("(%di128 * (pruv_fn as i128)) <= (pruv_budget as i128)" % len(plan.arrs))
        loads = "".join("let pruv_fx%d: i64 = unsafe { *%s.get_unchecked(pruv_fj) }; " % (k, x)
                        for k, x in enumerate(plan.arrs))
        ors = "".join("pruv_fbad |= !(%s); " % p for p in preds)
        return ("{ let pruv_fn: usize = %s.len(); if %s { let mut pruv_fc: ::std::vec::Vec<i64> = vec![0i64; pruv_fn]; "
                "let mut pruv_fbad = false; let mut pruv_fj: usize = 0; while pruv_fj < pruv_fn { %s%s"
                "unsafe { *pruv_fc.get_unchecked_mut(pruv_fj) = %s; } pruv_fj += 1; } "
                "if !pruv_fbad { return pruv_fc; } } }") % (
                    plan.n_arr, " && ".join(guards), loads, ors, s.fuse_elem(plan.elem, plan))

    # ---------- contract evaluation (requires), i128 ----------
    def carr(s, e):
        if isinstance(e, Var):   # R03L: an owned take array is read through a borrow
            return "(&%s)" % e.name if e.name in s.take_arrays else e.name
        if isinstance(e, StrLit):  # G2 B2
            return s.c(e)
        if isinstance(e, Call) and (e.name == "err_val" and e.ty == Type("string")
                                    or e.name in ("some_val", "ok_val") and e.borrowed_view):
            return s.c(e)
        if s.g9 and isinstance(e, (FieldAccess, VariantField)):   # G9: a buffer field of a real (extended) value
            return s.cx(e)
        raise Exception("codegen: unsupported array expression in contract: %r" % e)

    # ---------- G9: contract operands over REAL values ----------
    def g9_unsupported(s, e, what):
        return PruvError("codegen", e.line, e.col, "unsupported_type",
                         "%s is not supported in a runtime requires by the Rust backend yet" % what,
                         "compare tags with is_variant and fields with variant_field, or move the check into the body")

    def cx(s, e):
        """Rust expression of type `&R(T)` for a contract operand of nominal or buffer type, evaluated on the REAL
        parameter values (extended values have no i128 mirror, so nothing is converted eagerly and an inactive variant
        or a skipped operand is never inspected). Supported: a parameter, a field path, variant_field (a partial
        selector: `pruv_ud` when the variant is not active) and nullary constructors of a unit variant / empty struct."""
        if isinstance(e, Var):
            if e.name in s.bound or e.name not in s.params:
                raise s.g9_unsupported(e, "the variable %s as an aggregate operand" % e.name)
            return e.name if s.is_ext(e.ty) and e.name not in s.take_ext else "(&%s)" % e.name
        if isinstance(e, FieldAccess):
            deref = "*" if s.field_boxed(e.base.ty, e.field) else ""
            return "(&%s(%s).%s)" % (deref, s.cx(e.base), e.field)
        if isinstance(e, VariantField):
            if s.g9_owned(e.value):
                # the selected value is a fresh constructor (or a part of one): select from the OWNED temporary and
                # borrow the moved-out field, so no reference into a block-local temporary escapes its block
                return "(&%s)" % s.g9_owned_expr(e)
            s.tmp += 1
            tmp = "pruv_e%d" % s.tmp
            ety = Type(e.enum_name)
            vf = s.table.variant(ety, e.variant)
            idx = s.table.field_index(ety, e.field, e.variant)
            pats = ", ".join("pruv_f" if i == idx else "_" for i in range(len(vf.fields)))
            msg = "variant_field expected %s::%s.%s" % (getattr(e, "src_name", e.enum_name), e.variant, e.field)
            return ("({ let %s: &%s = %s; match %s { %s::%s(%s) => { let pruv_r: &%s = pruv_f; pruv_r } _ => pruv_ud(%s) } })" % (
                tmp, e.enum_name, s.cx(e.value), tmp, e.enum_name, e.variant, pats, rty(vf.fields[idx].type), rstr(msg)))
        if isinstance(e, Call) and e.name == "some_val" and isinstance(e.args[0], Call):
            lookup = e.args[0]
            if (s.is_ext(e.ty) and lookup.name == "map_lookup" and lookup.name not in s.fmap
                    and lookup.args[0].ty.args[0].name == "string"):
                return s.c(e)
        if isinstance(e, (EnumCtor, StructLit)):
            # a constructor (with or without payload) is a REAL temporary value of the type; a reference to it lives
            # through the enclosing statement. Contract integers are i128: every i64 field is converted with a checked
            # conversion (pruv_ud when out of range).
            return "(&%s)" % s.g9_real(e)
        raise s.g9_unsupported(e, "this expression as an aggregate operand")

    @staticmethod
    def g9_owned(e):
        """e denotes a fresh constructor value, or a field / variant_field part of one"""
        if isinstance(e, (EnumCtor, StructLit)):
            return True
        if isinstance(e, VariantField):
            return Gen.g9_owned(e.value)
        if isinstance(e, FieldAccess):
            return Gen.g9_owned(e.base)
        return False

    def g9_owned_expr(s, e):
        """OWNED Rust value of e (g9_owned(e)): a constructor is built (g9_real); a part is moved out of the owned
        value of its base (a Box edge is unboxed)"""
        if isinstance(e, (EnumCtor, StructLit)):
            return s.g9_real(e)
        if isinstance(e, FieldAccess):
            deref = "*" if s.field_boxed(e.base.ty, e.field) else ""
            return "(%s(%s).%s)" % (deref, s.g9_owned_expr(e.base), e.field)
        ety = Type(e.enum_name)
        vf = s.table.variant(ety, e.variant)
        idx = s.table.field_index(ety, e.field, e.variant)
        pats = ", ".join("pruv_f" if i == idx else "_" for i in range(len(vf.fields)))
        boxed = ("%s::%s.%s" % (e.enum_name, e.variant, e.field)) in s.boxed
        msg = "variant_field expected %s::%s.%s" % (getattr(e, "src_name", e.enum_name), e.variant, e.field)
        return "(match %s { %s::%s(%s) => %s, _ => pruv_ud(%s) })" % (
            s.g9_owned_expr(e.value), e.enum_name, e.variant, pats, "*pruv_f" if boxed else "pruv_f", rstr(msg))

    @staticmethod
    def g9_charge(n):
        """charge the shared contract budget n units (n: a Rust usize expression) before a buffer copy"""
        return ("{ let pruv_n: usize = %s; pruv_budget = pruv_budget.saturating_sub(i64::try_from(pruv_n).unwrap_or(i64::MAX)); "
                "if pruv_budget < 0 { pruv_ud(\"contract iteration budget exceeded\") } }" % n)

    def g9_field_real(s, x, ty, label):
        """a constructor field of declared type ty: the real value, boxed on a recursive edge"""
        v = s.g9_real(x, ty)
        return "::std::boxed::Box::new(%s)" % v if label in s.boxed else v

    def g9_real(s, e, ty=None):
        """Rust expression of the REAL (executed representation) OWNED value of the contract expression e (declared type
        ty, default e.ty). Constructors are built field by field in declaration order; i64 values come from the i128
        contract value through a checked conversion; named values and field paths are cloned from their reference."""
        if isinstance(e, StructLit):
            inits = {f.name: f.value for f in e.fields}
            fs = s.table.fields(Type(e.name))
            if not fs:
                return "%s {}" % e.name
            # R-6g9-2: a `ref` field borrows the contract buffer (Rust `&x`): no charge, no copy
            return "%s { %s }" % (e.name, ", ".join(
                "%s: %s" % (f.name, s.carr(inits[f.name]) if f.ref
                            else s.g9_field_real(inits[f.name], f.type, "%s.%s" % (e.name, f.name))) for f in fs))
        if isinstance(e, EnumCtor):
            ctor = "%s::%s" % (e.enum_name, e.variant)
            if not e.args:
                return ctor
            vf = s.table.variant(Type(e.enum_name), e.variant).fields
            return "%s(%s)" % (ctor, ", ".join(
                s.g9_field_real(a, fd.type, "%s::%s.%s" % (e.enum_name, e.variant, fd.name)) for a, fd in zip(e.args, vf)))
        t = ty if ty is not None else e.ty
        if isinstance(e, Ctor):
            px = t.args[0] if t is not None and t.args else I64
            rx = rty(px)
            if e.name == "None":
                return "None::<%s>" % rx
            if e.name == "Some":
                return "Some::<%s>(%s)" % (rx, s.g9_real(e.arg, px))
            err = result_error(t) if t is not None else I64
            if e.arg is None:   # LB8 R-6g7-1: Ok(())
                return "Ok::<(), %s>(())" % rty(err)
            return "%s::<%s, %s>(%s)" % (e.name, rx, rty(err), s.g9_real(e.arg, px if e.name == "Ok" else err))
        if t is None or (t.owner is None and t.name == "i64"):
            return "i64::try_from(%s).unwrap_or_else(|_| pruv_ud(\"constructor field out of i64 range\"))" % s.c(e)
        if t.owner is None and t.name == "bool":
            return s.c(e)
        if isinstance(e, (Var, FieldAccess, VariantField)) and (is_user_type(t) or t.name in ("option", "result")):
            if is_user_type(t) and not t.args and s.table.holds_buffer(t.name):
                raise s.g9_unsupported(e, "a copy of a value holding an array or str")
            # a copy of a named value / part is charged first: its weight (one unit per nominal node)
            w = s.g9_weight(t, "pruv_c", "&mut pruv_budget")
            if w == "0i128":
                return "::std::clone::Clone::clone(%s)" % s.cx(e)
            if nominals_in(t):
                s.need_size.update(nominals_in(t))
            return "({ let pruv_c: &%s = %s; let _ = %s; ::std::clone::Clone::clone(pruv_c) })" % (rty(t), s.cx(e), w)
        if is_array(t):
            # the buffer expression is evaluated once: bound, charged by its length, then copied
            return "({ let pruv_b: &[_] = %s; %s pruv_b.to_vec() })" % (s.carr(e), s.g9_charge("pruv_b.len()"))
        if t == STR:
            return "({ let pruv_b: &str = %s; %s pruv_b.to_owned() })" % (s.carr(e), s.g9_charge("pruv_b.len()"))
        raise s.g9_unsupported(e, "this expression as a constructor field of an aggregate operand")

    def g9_real_path(s, e):
        """e reads a part of a REAL (extended) value through references (cx): a variant_field of an extended value, or
        a field path below one"""
        if isinstance(e, VariantField):
            return s.is_ext(e.value.ty) or s.g9_real_path(e.value)
        if isinstance(e, FieldAccess):
            return s.is_ext(e.base.ty) or s.g9_real_path(e.base)
        return False

    def g9_metered(s, ty, ref):
        """contract value of type ty from `ref` (a Rust expression of type `&R(ty)`). A value holding nominal parts is
        converted whole, so the conversion is charged to the shared budget first: one unit per nominal node visited
        (the size helper ticks once per node)."""
        if not nominals_in(ty):
            return conv(ty, "(*%s)" % ref)
        s.need_size.update(nominals_in(ty))
        return "({ let pruv_m: &%s = %s; let _ = %s; %s })" % (
            rty(ty), ref, s.g9_weight(ty, "pruv_m", "&mut pruv_budget"), conv(ty, "(*pruv_m)"))

    def path_boxed(s, root, fields):
        """the LAST field of the assignment path root.f1.f2... is a Box edge (follows the declared field types)"""
        cur = s.vt.get(root)
        boxed = False
        for fname in fields:
            if cur is None or cur.owner is not None or cur.name not in s.table.decls:
                return False
            decl = s.table.decls[cur.name]
            if not isinstance(decl, StructDecl):
                return False
            boxed = ("%s.%s" % (cur.name, fname)) in s.boxed
            nxt = None
            for fd in decl.fields:
                if fd.name == fname:
                    nxt = fd.type
            cur = nxt
        return boxed

    def field_boxed(s, base_ty, field):
        """the field `field` of the struct type base_ty is a Box edge"""
        return bool(s.boxed) and base_ty is not None and ("%s.%s" % (base_ty.name, field)) in s.boxed

    def c(s, e):
        if isinstance(e, StrLit):  # G2 B2
            if e.empty_view:    # R-6g9-3: the static empty view, hand `&[]`
                return "&[]"
            literal = str_lit(e.value)
            return "(%s).as_bytes()" % literal if e.ty == ARRAY_U8 else literal
        if isinstance(e, IntLit):
            return "(%di128)" % e.value if e.value < 0 else "%di128" % e.value
        if isinstance(e, BoolLit):
            return "true" if e.value else "false"
        if isinstance(e, Var):
            if e.name in s.bound:
                return e.name
            t = e.ty.name if e.ty is not None else "i64"
            if s.is_ext(e.ty):    # G9: an extended value is only an operand of size / == / is_variant / field access
                raise s.g9_unsupported(e, "the extended value %s as a plain operand" % e.name)
            if s.cp is not None and e.ty is not None and is_user_type(e.ty) and e.name in s.params:
                s.cp[e.name] = e.ty
                return "pruv_cp_" + e.name
            name = "(*%s)" % e.name if e.name in s.inouts and t in ("i64", "bool") else e.name
            if is_owned_result(e.ty):
                return conv(e.ty, "%s.clone()" % name)
            if t in ("i64", "option", "result") or is_user_type(e.ty):
                return conv(e.ty if e.ty is not None else I64, name)
            return name
        if isinstance(e, Unary):
            if e.op in ("!", "~"):
                return "(!%s)" % s.c(e.operand)
            return "pruv_cneg(%s)" % s.c(e.operand)
        if isinstance(e, Binary):
            op = e.op
            if op in ("==", "!=") and s.is_ext(e.left.ty):
                # G9: structural equality of buffer-free extended aggregates, visited lazily on the real values; every
                # visited node pair costs one budget unit; only the active variant's fields are compared
                s.need_eq.add(e.left.ty.name)
                code = "pruv_ceq__%s(%s, %s, &mut pruv_budget)" % (e.left.ty.name, s.cx(e.left), s.cx(e.right))
                return code if op == "==" else "(!%s)" % code
            if op == "==>":
                return "(!%s || %s)" % (s.c(e.left), s.c(e.right))
            if op in CONTRACT_ARITH:
                return "%s(%s, %s)" % (CONTRACT_ARITH[op], s.c(e.left), s.c(e.right))
            if op in CONTRACT_OPS:
                s.need_bits.add(op)
                return "pruv_c%s_op(%s, %s)" % (CONTRACT_OPS[op], s.c(e.left), s.c(e.right))
            return "(%s %s %s)" % (s.c(e.left), op, s.c(e.right))
        if isinstance(e, Index):
            if e.array.ty is not None and e.array.ty.owner is None and e.array.ty.name == "vec":   # G6: checked read
                a = s.carr(e.array)
                return ("({ let pruv_i: i128 = %s; if pruv_i >= 0 && pruv_i < (%s.len() as i128) { "
                        "let pruv_x: %s = %s[pruv_i as usize]; %s } else { pruv_ud(\"vector index out of range\") } })") % (
                            s.c(e.index), a, rty(e.ty), a, conv(e.ty, "pruv_x"))
            if e.array.ty is not None and e.array.ty.owner is None and e.array.ty.name == "string":   # G6
                return "pruv_cidx_str(&%s, %s)" % (s.carr(e.array), s.c(e.index))
            if is_str(e.array.ty):  # G2 B2
                return "pruv_cidx_str(%s, %s)" % (s.carr(e.array), s.c(e.index))
            if is_bool_array(e.array.ty):  # G2 A5
                return "pruv_cidx_bool(%s, %s)" % (s.carr(e.array), s.c(e.index))
            if is_u8_array(e.array.ty):
                s.need_cidx_u8 = True
                return "pruv_cidx_u8(%s, %s)" % (s.carr(e.array), s.c(e.index))
            if s.layout(e.array) != "i64":
                return ('{ let pruv_a = %s; let pruv_i: i128 = %s; if pruv_i >= 0 && pruv_i < pruv_a.len() as i128 { pruv_a[pruv_i as usize] as i128 } else { pruv_ud("array index out of range") } }' % (s.carr(e.array), s.c(e.index)))
            return "pruv_cidx(%s, %s)" % (s.carr(e.array), s.c(e.index))
        if isinstance(e, Ctor):
            if e.ty is not None and e.ty.args:   # G3: Option<X> / Result<X>, X != i64
                cx = "()" if e.ty.args[0] == UNIT else cty(e.ty.args[0])  # LB8
                if e.name == "None":
                    return "None::<%s>" % cx
                view = e.name in ("Some", "Ok") and view_payload(e.ty) is not None
                arg = "()" if e.arg is None else s.carr(e.arg) if view else s.c(e.arg)
                if e.name == "Some":
                    return "Some::<%s>(%s)" % (cx, arg)
                error = result_error(e.ty)
                if e.name == "Err" and is_owned_result(e.ty):
                    arg = "%s.clone()" % arg
                return "%s::<%s, %s>(%s)" % (e.name, cx, cty(error), arg)
            if e.name == "None":
                return "None::<i128>"
            ctor = {"Some": "Some", "Ok": "Ok::<i128, i128>", "Err": "Err::<i128, i128>"}[e.name]
            return "%s(%s)" % (ctor, s.c(e.arg))
        if isinstance(e, Quant):
            return s.cquant(e)
        if isinstance(e, StructLit):
            # evaluated in declaration order (Rust evaluates literal fields in written order)
            inits = {f.name: f.value for f in e.fields}
            fs = s.table.fields(Type(e.name))
            body = ", ".join("%s: %s" % (f.name, s.c(inits[f.name])) for f in fs)
            return "(pruv_contract__%s { %s })" % (e.name, body) if fs else "(pruv_contract__%s {})" % e.name
        if isinstance(e, EnumCtor):
            ctor = "pruv_contract__%s::%s" % (e.enum_name, e.variant)
            if not e.args:
                return ctor
            return "%s(%s)" % (ctor, ", ".join(s.c(a) for a in e.args))
        if isinstance(e, FieldAccess):
            if s.is_ext(e.base.ty) or s.g9_real_path(e.base):
                # G9: a field of a real value (extended struct, or a part of one) is read from the original reference;
                # only the field itself is converted (a nominal field is converted whole and charged to the budget)
                if s.is_ext(e.ty) or is_array(e.ty) or e.ty == STR:
                    raise s.g9_unsupported(e, "the aggregate field %s as a plain operand" % e.field)
                if nominals_in(e.ty):
                    return s.g9_metered(e.ty, s.cx(e))
                return conv(e.ty, "(%s).%s" % (s.cx(e.base), e.field))
            return "%s.%s" % (s.c(e.base), e.field)
        if isinstance(e, IsVariant):
            if s.is_ext(e.value.ty) or s.g9_real_path(e.value):
                return "matches!(%s, %s::%s { .. })" % (s.cx(e.value), e.enum_name, e.variant)
            return "matches!(%s, pruv_contract__%s::%s { .. })" % (s.c(e.value), e.enum_name, e.variant)
        if isinstance(e, VariantField) and (s.is_ext(e.value.ty) or s.g9_real_path(e.value)):
            if s.is_ext(e.ty) or is_array(e.ty) or e.ty == STR:
                raise s.g9_unsupported(e, "the aggregate field %s as a plain operand" % e.field)
            s.tmp += 1
            tmp = "pruv_e%d" % s.tmp
            ety = Type(e.enum_name)
            vf = s.table.variant(ety, e.variant)
            idx = s.table.field_index(ety, e.field, e.variant)
            pats = ", ".join("pruv_f" if i == idx else "_" for i in range(len(vf.fields)))
            msg = "variant_field expected %s::%s.%s" % (getattr(e, "src_name", e.enum_name), e.variant, e.field)
            return "({ let %s: &%s = %s; match %s { %s::%s(%s) => %s, _ => pruv_ud(%s) } })" % (
                tmp, e.enum_name, s.cx(e.value), tmp, e.enum_name, e.variant, pats, s.g9_metered(e.ty, "pruv_f"), rstr(msg))
        if isinstance(e, VariantField):
            s.tmp += 1
            tmp = "pruv_e%d" % s.tmp
            cn = "pruv_contract__%s" % e.enum_name
            vf = s.table.variant(Type(e.enum_name), e.variant)
            idx = s.table.field_index(Type(e.enum_name), e.field, e.variant)
            pats = ", ".join("pruv_f" if i == idx else "_" for i in range(len(vf.fields)))
            msg = "variant_field expected %s::%s.%s" % (
                getattr(e, "src_name", e.enum_name) if s.explicit else e.enum_name, e.variant, e.field)   # G4: source-qualified
            return "({ let %s: %s = %s; match %s { %s::%s(%s) => pruv_f, _ => pruv_ud(%s) } })" % (
                tmp, cn, s.c(e.value), tmp, cn, e.variant, pats, rstr(msg))
        if isinstance(e, Call):
            n, a = e.name, e.args
            if n == "len":
                return "(%s.len() as i128)" % s.carr(a[0])
            if n == "size":   # also for a program without other G9 features (size of a plain acyclic struct / enum)
                # G9: structural size on the real value, checked i128 additions, one budget unit per node visited
                s.need_size.add(a[0].ty.name)
                return "pruv_csize__%s(%s, &mut pruv_budget)" % (a[0].ty.name, s.cx(a[0]))
            if n in ("file_ok", "file_readable", "file_writable"):
                return "(%s).%s" % (s.c(a[0]), n[5:])
            if n == 'ev_ok':
                return '((%s).error == 0)' % s.c(a[0])
            if n == 'ev_error':
                return '(((%s).error) as i128)' % s.c(a[0])
            if n == 'ev_cap':
                return '(((%s).public.len() as i64) as i128)' % s.c(a[0])
            if n == 'ev_count':
                return '(((%s).public.iter().filter(|p|p.kind!=0).count() as i64) as i128)' % s.c(a[0])
            if n == 'ev_last':
                return '(((%s).last) as i128)' % s.c(a[0])
            if n == 'ev_event_slot':
                return '(((%s).event_slot) as i128)' % s.c(a[0])
            if n == 'ev_event_kind':
                return '(((%s).event_kind) as i128)' % s.c(a[0])
            if n == 'ev_size':
                return '(((%s).buf.len() as i64) as i128)' % s.c(a[0])
            if n == 'ev_exit':
                return '(((%s).exit) as i128)' % s.c(a[0])
            if n == 'ev_kind':
                return '((%s).public[pruv_cev_slot(&(%s), %s)].kind as i128)' % (s.c(a[0]), s.c(a[0]), s.c(a[1]))
            if n == 'ev_eof':
                return '(%s).public[pruv_cev_slot(&(%s), %s)].eof' % (s.c(a[0]), s.c(a[0]), s.c(a[1]))
            if n == 'ev_write_open':
                return '(%s).public[pruv_cev_slot(&(%s), %s)].open' % (s.c(a[0]), s.c(a[0]), s.c(a[1]))
            if n == 'ev_byte':
                return '(pruv_cidx_u8(&(%s).buf, %s) as i128)' % (s.c(a[0]), s.c(a[1]))
            if n == "file_size":
                return "((%s).buf.len() as i128)" % s.c(a[0])
            if n == "file_byte":
                return "pruv_cidx_u8(&(%s).buf, %s)" % (s.c(a[0]), s.c(a[1]))
            # G10 proc A2: observers over the public state of the handle (never the OS or the queues)
            if n == "proc_ok":
                return "((%s).error == 0)" % s.c(a[0])
            if n == "proc_error":
                return "((%s).error as i128)" % s.c(a[0])
            if n == "proc_stdin_open":
                return "(%s).stdin_open" % s.c(a[0])
            if n == "proc_args_len":
                return "((%s).args.len() as i128)" % s.c(a[0])
            if n == "proc_eof":
                return "pruv_cproc_eof(&(%s), %s)" % (s.c(a[0]), s.c(a[1]))
            if n == "proc_size":
                return "(pruv_cproc_buf(&(%s), %s).len() as i128)" % (s.c(a[0]), s.c(a[1]))
            if n == "proc_byte":
                return "pruv_cproc_byte(&(%s), %s, %s)" % (s.c(a[0]), s.c(a[1]), s.c(a[2]))
            # G10 net A2: observers over the public state of the handle (never the OS)
            if n in ("net_ok", "net_listener_ok"):
                return "((%s).error == 0)" % s.c(a[0])
            if n in ("net_error", "net_listener_error"):
                return "((%s).error as i128)" % s.c(a[0])
            if n == "net_write_open":
                return "(%s).write_open" % s.c(a[0])
            if n == "net_eof":
                return "(%s).eof" % s.c(a[0])
            if n == "net_size":
                return "((%s).buf.len() as i128)" % s.c(a[0])
            if n == "net_byte":
                return "pruv_cnet_byte(&(%s), %s)" % (s.c(a[0]), s.c(a[1]))
            if n == "net_listener_port":
                return "((%s).port as i128)" % s.c(a[0])
            if n == "map_lookup":   # G6: finite-map lookup, Option mirror of the stored value
                kt, vt = a[0].ty.args[0], a[0].ty.args[1]
                key = s.carr(a[1]) if kt.name == "string" else s.c(a[1])
                if kt == I64:
                    look = "i64::try_from(%s).ok().and_then(|pruv_k| %s.get(&pruv_k).copied())" % (key, s.carr(a[0]))
                elif kt.name == "string":
                    if a[1].ty.name == "string" or isinstance(a[1], (FieldAccess, VariantField)):
                        key += ".as_str()"
                    look = "%s.get(%s)" % (s.carr(a[0]), key)
                    if s.is_ext(vt):
                        return look
                    look += ".copied()"
                else:
                    look = "%s.get(&%s).copied()" % (s.carr(a[0]), key)
                return "(%s)" % conv(mk_option(vt), "(%s)" % look)
            if n == "map_updated":   # G6: generated scan helper; shares the entry's iteration budget
                kt, vt = a[0].ty.args[0], a[0].ty.args[1]
                helper = s.fg.map_updated_helper(kt, vt, e.inst["src_args"] if e.inst else (kt, vt))
                return "%s(&%s, &%s, %s, %s, &mut pruv_budget)" % (helper, s.carr(a[0]), s.carr(a[1]),
                                                                  s.c(a[2]), s.c(a[3]))
            if n == "is_char_boundary":  # G2 B2
                return "pruv_cboundary(%s, %s)" % (s.carr(a[0]), s.c(a[1]))
            if n in ("load_be", "load_le"):
                if n == "load_le":
                    s.need_load_le = True
                else:
                    s.need_load_be = True
                return "pruv_c%s(%s, %s, %d)" % (n, s.carr(a[0]), s.c(a[1]), a[2].value)
            if n == "utf8_valid":
                return "::core::str::from_utf8(%s).is_ok()" % s.carr(a[0])
            if n == "utf8_valid_span":   # LG26 (R-6b-3): the --no-verify requires check; false outside the range
                return ("match (&%s, %s, %s) { (pruv_b, pruv_lo, pruv_hi) => pruv_lo >= 0 && pruv_lo <= pruv_hi && "
                        "pruv_hi <= (pruv_b.len() as i128) && "
                        "::core::str::from_utf8(&pruv_b[(pruv_lo as usize)..(pruv_hi as usize)]).is_ok() }"
                        % (s.carr(a[0]), s.c(a[1]), s.c(a[2])))
            if n in BIT_RUST:   # P2: i128 arguments checked inside i64 by the helper
                s.need_bits.add(n)
                return "pruv_c%s(%s, %s)" % (n, s.c(a[0]), s.c(a[1]))
            if n == "is_some":
                return "%s.is_some()" % s.c(a[0])
            if n == "some_val":
                return '%s.unwrap_or_else(|| pruv_ud("some_val of None"))' % s.c(a[0])
            if n == "is_ok":
                return "%s.is_ok()" % s.c(a[0])
            if n == "ok_val":
                return '%s.ok().unwrap_or_else(|| pruv_ud("ok_val of Err"))' % s.c(a[0])
            if n == "err_val":
                return '%s.err().unwrap_or_else(|| pruv_ud("err_val of Ok"))' % s.c(a[0])
        raise Exception("codegen: unsupported expression in requires: %r" % e)

    def cfast(s, e, var, j):
        """P3: a fast_forall body without branches (& and | instead of && and ||); array reads are unchecked because
        the prefilter guard has shown every read in bounds; j is the i64 loop counter holding var"""
        if isinstance(e, Unary):
            return "(!%s)" % s.cfast(e.operand, var, j)
        if isinstance(e, Binary) and e.op in ("&&", "||", "==>"):
            l, r = s.cfast(e.left, var, j), s.cfast(e.right, var, j)
            if e.op == "==>":
                return "((!%s) | %s)" % (l, r)
            return "(%s %s %s)" % (l, "&" if e.op == "&&" else "|", r)
        if isinstance(e, Binary):
            return "(%s %s %s)" % (s.cfast(e.left, var, j), e.op, s.cfast(e.right, var, j))
        if isinstance(e, Index):
            ix = e.index
            if isinstance(ix, Var):
                i = "%s as usize" % j
            else:
                i = "(%s %s %d) as usize" % (j, ix.op, ix.right.value)
            return "((unsafe { *%s.get_unchecked(%s) }) as i128)" % (e.array.name, i)
        return s.c(e)

    def cquant(s, e):
        arr = match_sorted_requires(e)
        if arr is not None and is_str(e.hi.args[0].ty):  # G2 B2: no sorted fast path for str
            arr = None
        if arr is not None:
            # O(n) equivalent (docs/M0_CONTRACTS.md C): forall pruv_sk in 1..len(a): a[pruv_sk-1] <= a[pruv_sk]
            L = dict(line=e.line, col=e.col)
            kv = Var(name="pruv_sk", **L)
            aty = e.hi.args[0].ty            # type of the original array variable
            ity = e.body.body.left.ty        # type of the original a[i] node
            e = Quant(kind="forall", var="pruv_sk", lo=IntLit(value=1, **L), hi=e.hi,
                      body=Binary(op="<=", left=Index(array=Var(name=arr, ty=aty, **L),
                                                       index=Binary(op="-", left=kv, right=IntLit(value=1, **L), **L),
                                                       ty=ity, **L),
                                  right=Index(array=Var(name=arr, ty=aty, **L), index=kv, ty=ity, **L), **L), **L)
        s.tmp += 1
        n = s.tmp
        lo, hi = s.c(e.lo), s.c(e.hi)   # bounds see only the outer scope
        s.bound.append(e.var)
        body = s.c(e.body)
        s.bound.pop()
        k, r, h = "pruv_k%d" % n, "pruv_r%d" % n, "pruv_hi%d" % n
        head = "{ let pruv_lo%d: i128 = %s; let %s: i128 = %s; let mut %s: i128 = pruv_lo%d; " % (n, lo, h, hi, k, n)
        loop = ("while %s < %s { pruv_budget -= 1; if pruv_budget < 0 { pruv_ud(\"contract iteration budget exceeded\"); } "
                "let %s: i128 = %s; ") % (k, h, e.var, k)
        tail = "%s += 1; } " % k
        kind = e.kind
        reads = fast_forall(e, lambda v: s.layout(v) == "i64" or is_u8_array(v.ty)) if s.verified else None
        if reads is not None:
            # P3: branch-free prefilter over the whole range (fast_forall); the scalar loop runs only when it fails
            s.bound.append(e.var)
            fb = s.cfast(e.body, e.var, "pruv_j%d" % n)
            s.bound.pop()
            lo_, hi_ = "pruv_lo%d" % n, h
            off = lambda x, c: x if c == 0 else "%s %s %di128" % (x, "+" if c > 0 else "-", abs(c))
            guards = ["%s >= -9223372036854775808i128" % lo_, "%s <= 9223372036854775807i128" % hi_,
                      "%s - %s <= (pruv_budget as i128)" % (hi_, lo_)]
            for a, c in reads:
                guards.append("%s >= 0i128" % off(lo_, c))
                guards.append("%s <= (%s.len() as i128)" % (off(hi_, c), a))
            fast = ("{ let mut pruv_a%d = true; let mut pruv_j%d: i64 = %s as i64; let pruv_e%d: i64 = %s as i64; "
                    "while pruv_j%d < pruv_e%d { let %s: i128 = pruv_j%d as i128; pruv_a%d &= %s; pruv_j%d += 1; } pruv_a%d }") % (
                        n, n, lo_, n, hi_, n, n, e.var, n, n, fb, n, n)
            return (head + "let mut %s = true; " % r + "if !(%s >= %s || (%s && %s)) { " % (lo_, hi_, " && ".join(guards), fast) +
                    loop + "if !(%s) { %s = false; break; } " % (body, r) + tail +
                    "} else if %s > %s { let pruv_b = &mut pruv_budget; *pruv_b -= (%s - %s) as i64; } " % (hi_, lo_, hi_, lo_) + r + " }")
        if kind == "forall":
            return head + "let mut %s = true; " % r + loop + "if !(%s) { %s = false; break; } " % (body, r) + tail + r + " }"
        if kind == "exists":
            return head + "let mut %s = false; " % r + loop + "if %s { %s = true; break; } " % (body, r) + tail + r + " }"
        if kind == "sum":
            return head + "let mut %s: i128 = 0; " % r + loop + "%s = pruv_cadd(%s, %s); " % (r, r, body) + tail + r + " }"
        if kind == "count":
            return head + "let mut %s: i128 = 0; " % r + loop + "if %s { %s += 1; } " % (body, r) + tail + r + " }"
        # max / min
        cmp = ">" if kind == "max" else "<"
        return (head + "let mut %s: ::std::option::Option<i128> = None; " % r + loop + "let pruv_v: i128 = %s; " % body +
                "%s = Some(match %s { None => pruv_v, Some(pruv_a) => if pruv_v %s pruv_a { pruv_v } else { pruv_a } }); " % (r, r, cmp) +
                tail + '%s.unwrap_or_else(|| pruv_ud("%s over an empty range")) }' % (r, kind))

    # ---------- G9: generated contract helpers (size, structural equality) ----------
    @staticmethod
    def decl_fields(d):
        return d.fields if isinstance(d, StructDecl) else [f for v in d.variants for f in v.fields]

    def g9_weight(s, t, x, bud="pruv_budget"):
        """Rust i128 expression for weight(value) (G9_SPEC 3); x names a variable of type `&R(t)`; bud is the budget
        argument expression (`pruv_budget` inside a helper, where it is a `&mut i64`).
        Scalars, arrays, str and None weigh 0; a nominal value weighs its size; Some / Ok / Err weigh their payload."""
        if s.rec_vec(t):
            return ("(%s).iter().fold(0i128, |pruv_sum, pruv_w| "
                    "pruv_cadd(pruv_sum, pruv_csize__%s(pruv_w, %s)))" % (x, t.args[0].name, bud))
        if t.owner is None and t.args and t.name in ("option", "result"):
            inner = s.g9_weight(t.args[0], "pruv_w", bud)
            err = s.g9_weight(t.args[1], "pruv_w", bud) if len(t.args) == 2 else "0i128"
            if inner == err == "0i128":
                return "0i128"
            if t.name == "option":
                return ("match %s { ::std::option::Option::Some(pruv_w) => %s, ::std::option::Option::None => 0i128 }"
                        % (x, inner))
            binder = "_" if err == "0i128" else "pruv_w"
            return ("match %s { ::std::result::Result::Ok(pruv_w) => %s, ::std::result::Result::Err(%s) => %s }"
                    % (x, inner, binder, err))
        if is_user_type(t):
            return "pruv_csize__%s(%s, %s)" % (t.name, x, bud)
        return "0i128"

    def g9_walks(s, t):
        """equality has to walk into t (a nominal value, or Option / Result of one)"""
        if t.owner is None and t.args and t.name in ("option", "result"):
            return any(s.g9_walks(a) for a in t.args)
        return is_user_type(t)

    def g9_eq(s, t, x, y):
        """Rust bool expression: structural equality of the values behind the references x and y (type `&R(t)`)"""
        if t.owner is None and t.args and t.name in ("option", "result") and s.g9_walks(t):
            inner = s.g9_eq(t.args[0], "pruv_p", "pruv_q")
            if t.name == "option":
                return ("(match (%s, %s) { (::std::option::Option::Some(pruv_p), ::std::option::Option::Some(pruv_q)) => %s, "
                        "(::std::option::Option::None, ::std::option::Option::None) => true, _ => false })" % (x, y, inner))
            err = s.g9_eq(result_error(t), "pruv_p", "pruv_q")
            return ("(match (%s, %s) { (::std::result::Result::Ok(pruv_p), ::std::result::Result::Ok(pruv_q)) => %s, "
                    "(::std::result::Result::Err(pruv_p), ::std::result::Result::Err(pruv_q)) => %s, "
                    "_ => false })" % (x, y, inner, err))
        if is_user_type(t):
            return "pruv_ceq__%s(%s, %s, pruv_budget)" % (t.name, x, y)
        return "(*%s == *%s)" % (x, y)

    def g9_size_text(s, d):
        """pruv_csize__Name: 1 + sum of the weights of the ACTIVE fields, one budget unit per call (= per node)"""
        n = d.name
        lines = [G9_HELPER_ALLOW, "fn pruv_csize__%s(v: &%s, pruv_budget: &mut i64) -> i128 {" % (n, n),
                 "    pruv_g9_tick(pruv_budget);"]

        def unit(fields):
            binds, ws = [], []
            for i, f in enumerate(fields):
                w = s.g9_weight(f.type, "pruv_a%d" % i)
                if w == "0i128":
                    binds.append("_")
                else:
                    binds.append("pruv_a%d" % i)
                    ws.append(w)
            return binds, ws

        def stmts(ws):
            """sequential accumulator statements (linear text, no nested pruv_cadd(pruv_cadd(..)..) chain)"""
            return ["let mut pruv_s: i128 = 1i128;"] + ["pruv_s = pruv_cadd(pruv_s, %s);" % w for w in ws]

        def fold(ws):
            if len(ws) == 0:
                return "1i128"
            if len(ws) == 1:
                return "pruv_cadd(1i128, %s)" % ws[0]
            return "{ %s pruv_s }" % " ".join(stmts(ws))
        if isinstance(d, StructDecl):
            binds, ws = unit(d.fields)
            if d.fields:
                lines.append("    let %s { %s } = v;" % (n, ", ".join("%s: %s" % (f.name, b) for f, b in zip(d.fields, binds))))
            if len(ws) >= 2:
                lines.extend("    " + x for x in stmts(ws))
                lines.append("    pruv_s")
            else:
                lines.append("    " + fold(ws))
        else:
            lines.append("    match v {")
            for v in d.variants:
                if not v.fields:
                    lines.append("        %s::%s => 1i128," % (n, v.name))
                    continue
                binds, ws = unit(v.fields)
                lines.append("        %s::%s(%s) => %s," % (n, v.name, ", ".join(binds), fold(ws)))
            lines.append("    }")
        lines.append("}")
        return "\n".join(lines) + "\n"

    def g9_eq_text(s, d):
        """pruv_ceq__Name: structural equality; fields are compared left to right and only for the same active variant"""
        n = d.name
        lines = [G9_HELPER_ALLOW, "fn pruv_ceq__%s(a: &%s, b: &%s, pruv_budget: &mut i64) -> bool {" % (n, n, n),
                 "    pruv_g9_tick(pruv_budget);"]

        def conj(fields):
            if not fields:
                return "true"
            return " && ".join(s.g9_eq(f.type, "pruv_a%d" % i, "pruv_b%d" % i) for i, f in enumerate(fields))
        if isinstance(d, StructDecl):
            if d.fields:
                for side, var in (("a", "pruv_a"), ("b", "pruv_b")):
                    lines.append("    let %s { %s } = %s;" % (n, ", ".join(
                        "%s: %s%d" % (f.name, var, i) for i, f in enumerate(d.fields)), side))
            lines.append("    " + conj(d.fields))
        else:
            lines.append("    match (a, b) {")
            for v in d.variants:
                if not v.fields:
                    lines.append("        (%s::%s, %s::%s) => true," % (n, v.name, n, v.name))
                    continue
                pa = ", ".join("pruv_a%d" % i for i in range(len(v.fields)))
                pb = ", ".join("pruv_b%d" % i for i in range(len(v.fields)))
                lines.append("        (%s::%s(%s), %s::%s(%s)) => %s," % (n, v.name, pa, n, v.name, pb, conj(v.fields)))
            lines.append("        _ => false,")
            lines.append("    }")
        lines.append("}")
        return "\n".join(lines) + "\n"

    def g9_helper_texts(s):
        """the helpers the runtime requires of this program need, in declaration order (size, then equality, per type).
        need_size / need_eq hold the operand types seen; both are closed over the nominal types of the fields."""
        if not (s.need_size or s.need_eq):
            return []
        decls = s.table.decls

        def close(seed):
            out = set()
            work = sorted(seed)
            while work:
                n = work.pop()
                if n in out or n not in decls:
                    continue
                out.add(n)
                for f in s.decl_fields(decls[n]):
                    if s.rec_vec(f.type):
                        work.append(f.type.args[0].name)
                    else:
                        work.extend(nominals_in(f.type))
            return out
        sizes, eqs = close(s.need_size), close(s.need_eq)
        texts = [G9_TICK]
        for d in decls.values():
            if d.name in sizes:
                texts.append(s.g9_size_text(d))
            if d.name in eqs:
                texts.append(s.g9_eq_text(d))
        return texts

    # ---------- G9: owning and viewing extended values (executable code) ----------
    def rec_vec(s, t):
        return (t.owner is None and t.name == "vec" and t.args[0].owner is None
                and not t.args[0].args and t.args[0].name in s.rec_vec_elems)

    def rec_vec_get(s, e):
        return (isinstance(e, Call) and e.name == "vec_get" and e.name not in s.fmap
                and s.rec_vec(e.args[0].ty))

    def rec_vec_view(s, e):
        s.fg.record_inline_catalog(e.name, tuple(e.inst.get("src_args", e.inst["args"])))
        v, i = map(s.e, e.args)
        if s.unchecked_catalog():
            return "(*unsafe { (%s).get_unchecked((%s) as usize) })" % (v, i)
        fail = ('pruv_g6_req("vec_get", "0 <= i && i < len(v)")' if s.fg is not None and s.fg.req
                else 'pruv_g6_repr("index")')
        index = "<usize as ::std::convert::TryFrom<i64>>::try_from(%s).unwrap_or(usize::MAX)" % i
        return "(*(%s).get(%s).unwrap_or_else(|| %s))" % (v, index, fail)

    def view(s, e):
        """Rust `&T` for an extended expression in a READ position (argument, match scrutinee): no clone, no alloc.
        A parameter / payload view is already a reference; an owned local or a field path is borrowed; a fresh temporary
        (call result, constructor) is borrowed as a temporary, which lives through the whole enclosing statement."""
        if isinstance(e, Var):
            return e.name if e.name in s.refs else "&" + e.name
        if isinstance(e, FieldAccess):
            return "&" + ("*" if s.field_boxed(e.base.ty, e.field) else "") + s.e(e)
        return "&(%s)" % s.e(e)

    def return_movable(s, e):
        """names of owned extended locals that the returned expression e may move instead of clone: a return is the
        last use of every local, so a local named exactly once in e (no other move or borrow of it in e) that is not a
        reference (parameter / payload view) and not the scrutinee of an enclosing match arm moves."""
        if e is None:
            return set()
        if not s.ext:
            # P2 batch 4: `return truncate(x, m)` of an owned local array shortens x in place in every program
            if (not s.cost and isinstance(e, Call) and e.name == "truncate" and len(e.args) == 2
                    and isinstance(e.args[0], Var) and e.args[0].name in s.vt and e.args[0].name not in s.params
                    and e.args[0].name not in s.refs and e.args[0].name not in s.inouts
                    and e.args[0].name not in s.pinned
                    and e.args[0].name not in s.borrowed_roots):
                m_names = set()
                _expr_names(e.args[1], m_names)
                if e.args[0].name not in m_names:
                    return {e.args[0].name}
            return set()
        counts = {}
        def walk(x):
            if isinstance(x, (Var, Move)):
                counts[x.name] = counts.get(x.name, 0) + 1
            for k in expr_kids(x):
                walk(k)
        walk(e)
        return {n for n, c in counts.items() if c == 1 and n not in s.refs and n not in s.params
                and n not in s.inouts and n not in s.pinned and n not in s.borrowed_roots}

    def own_e(s, e):
        """an expression in an OWNING position (let, assignment, constructor field, return). A named value or a field
        path is cloned explicitly; a fresh temporary (constructor, literal, call) moves. Other types: plain s.e.
        Inside a return, an owned local named once in the returned expression moves (return_movable)."""
        if s.ext and s.is_ext(e.ty) and isinstance(e, Var) and e.name in s.moving:
            return s.e(e)
        if s.ext and s.is_ext(e.ty) and isinstance(e, (Var, FieldAccess)):
            return "::std::clone::Clone::clone(%s)" % s.view(e)
        return s.e(e)

    def field_value(s, x, label):
        """the value stored into the field `label` of a constructor / literal: owned, boxed on a recursive edge"""
        v = s.own_e(x)
        return "::std::boxed::Box::new(%s)" % v if label in s.boxed else v

    # ---------- statements / code expressions ----------
    def block(s, stmts, ind, decls=(), loop=False):
        saved = dict(s.vt)
        saved_refs = set(s.refs)
        saved_inouts = set(s.inouts)
        later = [set() for _ in range(len(stmts) + 1)]
        for i in range(len(stmts) - 1, -1, -1):
            later[i] = set(later[i + 1])
            _stmt_names([stmts[i]], later[i])
        frame = [set(decls), set(), loop, False]
        s.frames.append(frame)
        for i, st in enumerate(stmts):
            frame[1] = later[i + 1]
            frame[3] = i + 1 < len(stmts) and isinstance(stmts[i + 1], Return)
            s.stmt(st, ind)
            if id(st) in s.exits:
                s.emit(ind, "break;")
            if isinstance(st, Let):
                frame[0].add(st.name)
        s.frames.pop()
        s.vt = saved
        s.refs = saved_refs
        s.inouts = saved_inouts

    def dead_after(s, name):
        """the statement being emitted is the last use of local `name`: no later statement of any enclosing block up to
        the one declaring it names it, and no loop body lies between (a parameter or binder view is never dead here)."""
        if name in s.refs or name in s.inouts or name in s.params or name in s.pinned or name in s.borrowed_roots:
            return False
        for decls, later, loop, _ in reversed(s.frames):
            if name in later:
                return False
            if name in decls:
                return True
            if loop:
                return False
        return False

    def return_next(s, name):
        """R03L: the statement after the one being emitted is a return of the same block that does not name local
        `name`, so `name` is dead even inside a loop body (control leaves the function before any later iteration)."""
        if name in s.refs or name in s.inouts or name in s.params or name in s.pinned or name in s.borrowed_roots or not s.frames:
            return False
        return s.frames[-1][3] and name not in s.frames[-1][1]

    def last_copy(s, st):
        """a let / assignment whose value is copy(v) of an owned local v at its last use moves v (see dead_after)"""
        e = st.value
        # R03L: truncate of a dead owned local moves in every program (the tape handed to a take parameter)
        if (not s.cost and isinstance(e, Call) and e.name == "truncate" and len(e.args) == 2
                and isinstance(e.args[0], Var) and e.args[0].name in s.vt and e.args[0].name != st.name
                and (s.dead_after(e.args[0].name) or s.return_next(e.args[0].name))):
            # the length argument naming the array again (e.g. len(truncate(a, 1))) keeps it borrowed: no move
            m_names = set()
            _expr_names(e.args[1], m_names)
            return set() if e.args[0].name in m_names else {e.args[0].name}
        # LG33 (G-DB54-2): str_from_utf8 of a dead owned local [u8] moves the vector into the String (no second copy)
        if (not s.cost and isinstance(e, Call) and e.name == "str_from_utf8" and len(e.args) == 1
                and isinstance(e.args[0], Var) and e.args[0].name in s.vt and e.args[0].name != st.name
                and (s.dead_after(e.args[0].name) or s.return_next(e.args[0].name))):
            return {e.args[0].name}
        if not (s.ext and not s.cost and isinstance(e, Call) and e.name == "copy" and len(e.args) == 1):
            return set()
        x = e.args[0]
        if x.borrowed_view:
            return set()
        if isinstance(x, Var) and x.name != st.name and s.dead_after(x.name):
            return {x.name}
        # copy(v.f) of an unboxed field of an owned local v at its last use moves the field out of v
        if (isinstance(x, FieldAccess) and isinstance(x.base, Var) and x.base.name != st.name
                and not s.field_boxed(x.base.ty, x.field) and s.dead_after(x.base.name)):
            return {x.base.name}
        return set()

    def stack_elem(s, st):
        """P3-S1: Rust element type of a stack array declaration"""
        if is_bool_array(st.type):
            return "bool"
        if is_u8_array(st.type):
            return "u8"
        return s.layout(st)

    def stack_type(s, st):
        return "[%s; %d]" % (s.stack_elem(st), s.stack.length(st))

    def stack_init(s, st):
        """P3-S1: fixed-size array initializer; the fill value is evaluated first, once (n is a literal)"""
        n = s.stack.length(st)
        v = st.value.args[1]
        elem = s.stack_elem(st)
        if elem == "bool":
            return "{ let pruv_v: bool = %s; [pruv_v; %d] }" % (s.e(v), n)
        chk = ""
        if elem == "u8" and not s.verified:
            chk = "if pruv_v < 0 || pruv_v > 255 { pruv_elem_oor(pruv_v) } "
        cast = "" if elem == "i64" else " as " + elem
        return "{ let pruv_v: i64 = %s; %s[pruv_v%s; %d] }" % (s.e(v), chk, cast, n)

    def stmt(s, st, ind):
        if isinstance(st, Let) and s.stack is not None and s.stack.contains(st) and st.name not in s.resize_args:
            s.emit(ind, "let %s%s: %s = %s;" % ("mut " if st.mutable else "", st.name, s.stack_type(st), s.stack_init(st)))
            s.vt[st.name] = st.type
            s.refs.discard(st.name)
        elif isinstance(st, Let) and s.small_o is not None and st.name == s.small_o:
            a, el = st.value.args, s.small_elem
            cast = "" if el == "i64" else " as " + el
            s.emit(ind, "let %s: &mut [%s] = { let pruv_n: i64 = %s; let pruv_v: i64 = %s; let pruv_s: &mut [%s] = "
                        "&mut pruv_sb[..pruv_n as usize]; pruv_s.fill(pruv_v%s); pruv_s };"
                   % (st.name, el, s.e(a[0]), s.e(a[1]), el, cast))
            s.vt[st.name] = st.type
            s.refs.discard(st.name)
        elif isinstance(st, Let) and id(st) in s.small_sites:
            _, k, el = s.small_sites[id(st)]
            s.small_call, s.small_buf = st.value, st.name
            call = s.e(st.value)
            s.small_call = None
            s.emit(ind, "let mut pruv_sb_%s: [%s; %d] = [0%s; %d];" % (st.name, el, k, el, k))
            s.emit(ind, "let pruv_sl_%s: usize = %s;" % (st.name, call))
            s.emit(ind, "let %s: &[%s] = &pruv_sb_%s[..pruv_sl_%s];" % (st.name, el, st.name, st.name))
            s.vt[st.name] = st.type
            s.refs.discard(st.name)
        elif isinstance(st, Let) and st.name in s.static_strs:   # LB9 SL (R-6g8-3): `let x = "lit";`
            s.emit(ind, "let %s: &'static str = %s;" % (st.name, str_lit(st.value.value)))
            s.vt[st.name] = st.type
            s.refs.add(st.name)
        elif isinstance(st, Let) and isinstance(st.value, InoutArg):   # R-6g9-5: `let d = inout P;` is `&mut P`
            s.emit(ind, "let %s%s = %s;" % ("mut " if st.mutable else "", st.name, s.place_ref(st.value.expr)))
            s.vt[st.name] = st.type
            s.refs.discard(st.name)
            s.inouts.add(st.name)
        elif isinstance(st, Let):
            s.moving = s.last_copy(st)
            binding = ("mut " if st.mutable else "") + st.name
            if not (isinstance(st.value, Move) and st.value.fields):
                binding += ": " + (rty(st.type, True) if st.ref else s.rt(st.type, site=st))
            value = s.arr_ref(st.value) if st.ref else s.own_e(st.value)
            s.emit(ind, "let %s = %s;" % (binding, value))
            s.moving = set()
            s.vt[st.name] = st.type
            if st.ref:
                s.refs.add(st.name)
            else:
                s.refs.discard(st.name)
        elif isinstance(st, Assign):
            s.moving = s.last_copy(st)
            call = st.value
            if getattr(call, "resize_scratch", False):
                resize = s.resize_expr(call, st.name)
                if s.cost:
                    resize = "{ %s %s }" % (resize, s.cost_add("1u128 + %s.len() as u128" % st.name))
                s.emit(ind, resize)
                return
            if (not s.cost and not s.moving and isinstance(call, Call) and call.name in ("resize", "truncate")
                    and call.name not in s.fmap and st.name not in s.refs
                    and (st.name not in s.params or st.name in s.take_arrays or st.name in s.resizable)):
                source = call.args[0]
                if (isinstance(source, (Var, Move)) and source.name == st.name
                        and not getattr(source, "fields", ()) and is_array(source.ty)):
                    s.emit(ind, s.array_resize_expr(call, st.name))
                    return
            target = "*" + st.name if st.name in s.inouts else st.name
            s.emit(ind, "%s = %s;" % (target, s.own_e(st.value)))
            s.moving = set()
        elif isinstance(st, FieldAssign):
            vty = st.value.ty
            call = st.value
            if st.ref_store and vty in VIEW_BASES:
                # R-6g9-6: store a slice or str view directly, without an owned temporary
                s.emit(ind, "%s.%s = %s;" % (st.name, ".".join(st.fields), s.arr_ref(st.value)))
                return
            if (isinstance(call, Call) and (call.name == "vec_push"
                    or call.name == "map_insert" and (is_nested_collection_field(vty) or is_string_record_map(vty)))
                    and call.name not in s.fmap and isinstance(call.args[0], Move)
                    and call.args[0].fields and s.unchecked_catalog()):
                args = tuple(call.inst.get("src_args", call.inst["args"]))
                if call.name == "vec_push":
                    s.fg.record_inline_catalog(call.name, args)
                    s.emit(ind, "%s.%s.push(%s);" % (st.name, ".".join(st.fields), s.e(call.args[1])))
                    return
                s.fg.record_inline_catalog(call.name, args)
                s.emit(ind, "%s.%s.insert(%s, %s);" %
                       (st.name, ".".join(st.fields), s.e(call.args[1]), s.e(call.args[2])))
                return
            if (not s.cost and isinstance(call, Call) and call.name in ("resize", "truncate") and call.name not in s.fmap
                    and is_array(vty) and same_field_path(call.args[0], st.name, st.fields)
                    and not (s.boxed and s.path_boxed(st.name, st.fields))):
                # LG34 (G-LG32-3): `x.f = resize(x.f, n, v)` grows the field in place, like the plain-name assign
                s.emit(ind, s.array_resize_expr(call, "%s.%s" % (st.name, ".".join(st.fields))))
                return
            if (vty is not None and is_collection(vty)) or (st.name in s.inouts and not (s.boxed and s.path_boxed(st.name, st.fields))):
                s.emit(ind, "%s.%s = %s;" % (st.name, ".".join(st.fields), s.own_e(st.value) if vty is None or not is_collection(vty) else s.e(st.value)))
                return
            # rhs is fully evaluated into a temporary before the store (the replaced value drops at the store)
            decl = "let pruv_v: %s = " % rty(vty) if vty is not None else "let pruv_v = "
            stored = "pruv_v"
            if s.boxed and s.path_boxed(st.name, st.fields):
                stored = "::std::boxed::Box::new(pruv_v)"    # the assigned field is a recursive (Box) edge
            s.emit(ind, "{ %s%s; %s.%s = %s; }" % (decl, s.own_e(st.value), st.name, ".".join(st.fields), stored))
        elif isinstance(st, IndexAssign) and st.fields:
            # LG4: `p.a.b[i] = e` on the place `p.a.b` (auto-deref through `&mut S` and Box edges)
            place = "%s.%s" % (st.name, ".".join(st.fields))
            fty = s.vt.get(st.name)
            for fname in st.fields:
                decl = s.table.decls.get(fty.name) if fty is not None else None
                fty = next((fd.type for fd in decl.fields if fd.name == fname), None) if isinstance(decl, StructDecl) else None
            if is_bool_array(fty):
                if s.verified:
                    s.emit(ind, "{ let pruv_i: i64 = %s; let pruv_x: bool = %s; unsafe { *%s.get_unchecked_mut(pruv_i as usize) = pruv_x; } }" %
                           (s.e(st.index), s.e(st.value), place))
                else:
                    s.emit(ind, "{ let pruv_i: i64 = %s; let pruv_x: bool = %s; pruv_set_bool(&mut %s, pruv_i, pruv_x); }" %
                           (s.e(st.index), s.e(st.value), place))
            elif is_u8_array(fty):
                if s.verified:
                    s.emit(ind, "{ let pruv_i: i64 = %s; let pruv_x: i64 = %s; unsafe { *%s.get_unchecked_mut(pruv_i as usize) = pruv_x as u8; } }" %
                           (s.e(st.index), s.e(st.value), place))
                else:
                    s.emit(ind, "{ let pruv_i: i64 = %s; let pruv_x: i64 = %s; pruv_set_u8(&mut %s, pruv_i, pruv_x); }" %
                           (s.e(st.index), s.e(st.value), place))
            elif s.verified:
                s.emit(ind, "{ let pruv_i: i64 = %s; let pruv_x: i64 = %s; unsafe { *%s.get_unchecked_mut(pruv_i as usize) = pruv_x; } }" %
                       (s.e(st.index), s.e(st.value), place))
            else:
                s.emit(ind, "{ let pruv_i: i64 = %s; let pruv_x: i64 = %s; pruv_set(&mut %s, pruv_i, pruv_x); }" %
                       (s.e(st.index), s.e(st.value), place))
        elif isinstance(st, IndexAssign):
            if is_bool_array(s.vt.get(st.name)):  # G2 A5
                if s.verified:
                    s.emit(ind, "{ let pruv_i: i64 = %s; let pruv_x: bool = %s; unsafe { *%s.get_unchecked_mut(pruv_i as usize) = pruv_x; } }" %
                           (s.e(st.index), s.e(st.value), st.name))
                else:
                    s.emit(ind, "{ let pruv_i: i64 = %s; let pruv_x: bool = %s; pruv_set_bool(%s, pruv_i, pruv_x); }" %
                           (s.e(st.index), s.e(st.value), s.mut_ref(st.name)))
            elif is_u8_array(s.vt.get(st.name)):
                if s.verified:
                    s.emit(ind, "{ let pruv_i: i64 = %s; let pruv_x: i64 = %s; unsafe { *%s.get_unchecked_mut(pruv_i as usize) = pruv_x as u8; } }" %
                           (s.e(st.index), s.e(st.value), st.name))
                else:
                    s.emit(ind, "{ let pruv_i: i64 = %s; let pruv_x: i64 = %s; pruv_set_u8(%s, pruv_i, pruv_x); }" %
                           (s.e(st.index), s.e(st.value), s.mut_ref(st.name)))
            elif s.verified and s.layout(st) != "i64":
                s.emit(ind, "{ let pruv_i: i64 = %s; let pruv_x: i64 = %s; unsafe { *%s.get_unchecked_mut(pruv_i as usize) = pruv_x as %s; } }" %
                       (s.e(st.index), s.e(st.value), st.name, s.layout(st)))
            elif s.verified:
                s.emit(ind, "{ let pruv_i: i64 = %s; let pruv_x: i64 = %s; unsafe { *%s.get_unchecked_mut(pruv_i as usize) = pruv_x; } }" %
                       (s.e(st.index), s.e(st.value), st.name))
            else:
                # temporaries first: `&mut a` must not be live while index/value read `a`
                s.emit(ind, "{ let pruv_i: i64 = %s; let pruv_x: i64 = %s; pruv_set(%s, pruv_i, pruv_x); }" %
                       (s.e(st.index), s.e(st.value), s.mut_ref(st.name)))
        elif isinstance(st, If):
            s.emit(ind, "if %s {" % s.e(st.cond))
            prev, s.utf8_ok = s.utf8_ok, utf8_checked(st)
            s.block(st.then, ind + 1)
            s.utf8_ok = prev
            if st.els:
                s.emit(ind, "} else {")
                s.block(st.els, ind + 1)
            s.emit(ind, "}")
        elif isinstance(st, While):
            s.emit(ind, "while %s {" % s.e(st.cond))
            if s.cost:
                s.emit(ind + 1, s.cost_add("1"))
            exits = set() if s.cost else loop_exits(st)
            s.exits |= exits
            try:
                s.block(st.body, ind + 1, loop=True)
            finally:
                s.exits -= exits
            s.emit(ind, "}")
        elif isinstance(st, ForMap):
            key_ty, val_ty = st.place.ty.args
            scalar = val_ty in (I64, BOOL)
            s.emit(ind, "for (%s, %s%s) in %s.iter() {" %
                   (st.key, "&" if scalar else "", st.val, s.e(st.place)))
            saved_refs, saved_vt = set(s.refs), dict(s.vt)
            s.vt.update({st.key: key_ty, st.val: val_ty})
            s.refs.add(st.key)
            if not scalar:
                s.refs.add(st.val)
            s.block(st.body, ind + 1, loop=True)
            s.refs, s.vt = saved_refs, saved_vt
            s.emit(ind, "}")
        elif isinstance(st, Return) and s.small_o is not None and isinstance(st.value, Var) and st.value.name == s.small_o:
            s.emit(ind, "return %s.len();" % s.small_o)
        elif isinstance(st, Return):
            s.moving = s.return_movable(st.value)
            value = None if st.value is None else (s.arr_ref(st.value) if s.ref_result else s.own_e(st.value))
            if s.cost:
                if value is not None:
                    s.emit(ind, "let pruv_return = %s;" % value)
                s.moving = set()
                s.emit(ind, s.cost_check())
                s.emit(ind, "return;" if st.value is None else "return pruv_return;")
                return
            s.emit(ind, "return;" if value is None else "return %s;" % value)
            s.moving = set()
        elif isinstance(st, Assert):
            pass
        elif isinstance(st, ExprStmt):
            s.emit(ind, s.e(st.expr) + ";")
        elif isinstance(st, Par):
            s.parallel(st, ind)
        elif isinstance(st, Match) and s.is_ext(st.scrutinee.ty):
            s.match_ext(st, ind)
        elif isinstance(st, Match) and s.try_match(st):
            s.try_stmt(st, ind)
        elif isinstance(st, Match):
            sc = st.scrutinee
            from foreign import MUTABLE_COLLECTION_MATCH
            catalog_match = isinstance(sc, Call) and sc.name not in s.fmap
            collection_call = catalog_match and sc.name in MUTABLE_COLLECTION_MATCH + ("map_get",)
            map_take = catalog_match and sc.name == "map_take"
            ty = sc.inst["args"][-1] if collection_call or map_take else None
            mutable_map = collection_call and sc.name in MUTABLE_COLLECTION_MATCH
            map_view = collection_call and s.is_ext(ty)
            if collection_call and sc.name == "map_get" and is_string_record_map(sc.args[0].ty):
                root, path = field_place(sc.args[0])
                map_view |= bool(path and isinstance(root, Var) and root.name in s.inouts and root.name not in s.params)
            scoped = False
            if mutable_map or map_view:
                from foreign_shim import inst_types
                s.fg.record_inline_catalog(sc.name, inst_types(sc)[1])
                place, key = sc.args
                if mutable_map:
                    place = place.expr
                receiver = s.e(place)
                if not isinstance(place, (Var, FieldAccess)):
                    receiver = "(%s)" % receiver
                text_key = sc.name != "vec_at_mut" and sc.inst["args"][0].name == "string"
                key_text = s.borrowed_map_key(key) if text_key else s.e(key)
                simple_key = (Var, FieldAccess, StrLit) if text_key else (Var, IntLit, BoolLit)
                scoped = mutable_map and not isinstance(key, simple_key)
                if scoped:
                    s.emit(ind, "{")
                    ind += 1
                    s.emit(ind, "let pruv_key = %s;" % key_text)
                    key_text = "pruv_key"
                if sc.name == "vec_at_mut":
                    key_text = "<usize as ::std::convert::TryFrom<i64>>::try_from(%s).unwrap_or(usize::MAX)" % key_text
                elif not text_key:
                    key_text = "&" + key_text
                scrutinee = "%s.%s(%s)" % (receiver, "get_mut" if mutable_map else "get", key_text)
            else:
                scrutinee = s.e(sc)
            s.emit(ind, "match %s {" % scrutinee)
            for a in st.arms:
                name = a.ctor if a.enum_name is None else "%s::%s" % (a.enum_name, a.ctor)
                binders = ", ".join(a.binders)
                if map_take and name == "Some":
                    binders = "mut " + binders
                pat = name if not a.binders else "%s(%s)" % (name, binders)
                if name == "Ok" and not a.binders:   # LB8 R-6g7-1: the unit payload
                    pat = "Ok(())"
                s.emit(ind + 1, "%s => {" % pat)
                saved_inouts, saved_refs, saved_vt = set(s.inouts), set(s.refs), dict(s.vt)
                if a.ctor in ("Some", "Ok") and a.binders and (base := view_payload(sc.ty)) is not None:
                    b = a.binders[0]
                    s.vt[b] = base
                    s.refs.add(b)
                elif (collection_call or map_take) and a.binders:
                    b = a.binders[0]
                    s.vt[b] = ty
                    if mutable_map:
                        s.inouts.add(b)
                    if mutable_map or map_view:
                        s.refs.add(b)
                s.block(a.body, ind + 2)
                s.inouts, s.refs, s.vt = saved_inouts, saved_refs, saved_vt
                s.emit(ind + 1, "}")
            s.emit(ind, "}")
            if scoped:
                s.emit(ind - 1, "}")

    def try_match(s, st):
        """the checker's lowering of `e?` (check.try_arms): `match e { Ok(v) => { rest } Err(__pruv_try_eN) =>
        { return Err(__pruv_try_eN); } }` with the same error type, where the value is unit or the first statement
        of rest takes v whole (`let x: T = v;` / `x = v;`). Not under cost metering (its return adds a check)."""
        if s.cost or len(st.arms) != 2:
            return False
        ok, fail = st.arms
        if ok.enum_name is not None or fail.enum_name is not None or ok.ctor != "Ok" or fail.ctor != "Err":
            return False
        if len(fail.binders) != 1 or not fail.binders[0].startswith("__pruv_try_e") or len(fail.body) != 1:
            return False
        ret = fail.body[0]
        if not (isinstance(ret, Return) and isinstance(ret.value, Ctor) and ret.value.name == "Err"
                and isinstance(ret.value.arg, Var) and ret.value.arg.name == fail.binders[0]):
            return False
        if not ok.binders:
            return True
        first = ok.body[0] if ok.body else None
        if isinstance(first, Let) and (getattr(first, "ref", False) or (s.stack is not None and s.stack.contains(first))):
            return False
        return (isinstance(first, (Let, Assign)) and isinstance(first.value, Var) and first.value.name == ok.binders[0]
                and not s.is_ext(first.value.ty))

    def try_stmt(s, st, ind):
        """`e?` as the hand-written Rust `?` (`From` is the identity: try_arms requires the same error type). The Ok
        arm holds the rest of the block, so it follows at the same level: `f(..)?;` for a unit Ok, else the first
        statement of the rest takes `f(..)?` in place of the binder (`let x: T = f(..)?;` / `x = f(..)?;`)."""
        ok = st.arms[0]
        value = "%s?" % s.e(st.scrutinee)
        if ok.binders:
            s.try_values[ok.binders[0]] = value
        else:
            s.emit(ind, value + ";")
        s.block(ok.body, ind)

    def match_ext(s, st, ind):
        """match on an extended enum: the scrutinee is borrowed (a view, never cloned) and every payload binder is an
        arm-local view, unless the match consumes the scrutinee (below). Scalars and non-extended values are copied out (`let v: i64 = *v;`), extended
        payloads become `&T` (a Box edge derefs by coercion), buffers become `&[T]` / `&str`."""
        sty = st.scrutinee.ty
        # Consuming match: a match on an owned local that no arm and no later statement names again (dead_after) is
        # that local's last use, so it is matched by value and payloads bind as owned values a return or copy moves.
        sc = st.scrutinee
        # G-DB16-1: an arm that moves an array binder (`move tape`) consumes the scrutinee, also under cost metering
        consume = isinstance(sc, Var) and (match_moves_binder(st) or (not s.cost and s.dead_after(sc.name)))
        if consume:
            named = set()
            for a in st.arms:
                named.update(a.binders)
                _stmt_names(a.body, named)
            consume = consume and sc.name not in named
        s.emit(ind, "match %s {" % (s.e(sc) if consume else s.view(sc)))
        for a in st.arms:
            name = a.ctor if a.enum_name is None else "%s::%s" % (a.enum_name, a.ctor)
            pat = name if not a.binders else "%s(%s)" % (name, ", ".join(a.binders))
            s.emit(ind + 1, "%s => {" % pat)
            saved_vt, saved_refs, saved_pinned = dict(s.vt), set(s.refs), set(s.pinned)
            saved_owned = set(s.owned_binders)
            root = field_place(sc)[0]
            if s.rec_vec_get(root):
                root = field_place(root.args[0])[0]
            if isinstance(root, Var):
                s.pinned.add(root.name)
            fields = s.table.variant(sty, a.ctor).fields
            for b, fd in zip(a.binders, fields):
                ft = fd.type
                if s.is_ext(ft) and consume:
                    boxed = ("%s::%s.%s" % (sty.name, a.ctor, fd.name)) in s.boxed
                    s.emit(ind + 2, "let %s: %s = %s%s;" % (b, ft.name, "*" if boxed else "", b))
                    s.refs.discard(b)
                    s.owned_binders.add(b)    # LB9 (R-6g8-4): `move b.f` is the plain partial move `b.f`
                elif s.is_ext(ft):
                    s.emit(ind + 2, "let %s: &%s = %s;" % (b, ft.name, b))
                    s.refs.add(b)
                elif is_owned_field_type(ft) or s.rec_vec(ft):
                    s.emit(ind + 2, "let %s: %s = %s%s;" % (b, rty(ft, True), "&" if consume else "", b))
                    s.refs.add(b)
                elif (is_array(ft) or ft == STR) and consume:
                    s.emit(ind + 2, "let %s: %s = %s;" % (b, rty(ft), b))
                    s.refs.discard(b)
                elif is_array(ft) or ft == STR:
                    s.emit(ind + 2, "let %s: %s = %s;" % (b, rty(ft, True), b))
                    s.refs.add(b)
                elif consume:
                    s.emit(ind + 2, "let %s: %s = %s;" % (b, rty(ft), b))
                    s.refs.discard(b)
                else:
                    s.emit(ind + 2, "let %s: %s = *%s;" % (b, rty(ft), b))
                    s.refs.discard(b)
                s.vt[b] = ft
            s.block(a.body, ind + 2, decls=a.binders)
            s.vt, s.refs, s.pinned = saved_vt, saved_refs, saved_pinned
            s.owned_binders = saved_owned
            s.emit(ind + 1, "}")
        s.emit(ind, "}")

    def parallel(s, st, ind):
        s.emit(ind, "{")
        hook = s.par_hook in ("spawn", "panic", "overlap")
        if hook:
            # Workers rendezvous after spawning; expected count is the actual number started,
            # so even an unexpected OS spawn failure cannot deadlock the test gate.
            s.emit(ind + 1, "let pruv_gate = &(::std::sync::Mutex::new((false, 0usize, 0usize)), ::std::sync::Condvar::new());")
        ind += 1
        s.emit(ind, "::std::thread::scope(|pruv_scope| {")
        d = ind + 1
        prepared = []
        for i, call in enumerate(st.calls):
            args = []
            for j, x in enumerate(call.args):
                n = "pruv_arg_%d_%d" % (i, j)
                if isinstance(x, InoutArg):
                    name = x.expr.name
                    actual = s.vt[name]
                    replaces = (call.name, j) in s.array_resizes
                    ty = "&mut " + (rty(actual, True)[1:] if is_array(actual) and not replaces else rty(actual))
                    val = (s.inout_ref(x.expr, call.name, j) if replaces or name in s.resizable else
                           "&mut *" + name if name in s.inouts else s.mut_ref(name))
                elif is_array(x.ty):
                    ty, val = s.rt(x.ty, True, x), s.arr_ref(x)
                else:
                    ty, val = rty(x.ty), s.e(x)
                s.emit(d, "let %s: %s = %s;" % (n, ty, val))
                args.append(n)
            n = s.callee(call.name) if s.explicit else call.name
            if not s.explicit and call.name in s.body_named:
                if s.verified:
                    n += "__pruv_body"
                elif call.name in s.has_req and s.trust and not any(isinstance(x, InoutArg) for x in call.args):
                    n += "__pruv_par_chk"
            if s.cost:
                s.emit(d, s.cost_call(call.name, args))
            prepared.append("%s(%s);" % (n, ", ".join(args)))
        s.emit(d, "let mut pruv_handles = ::std::vec::Vec::new();")
        s.emit(d, "let mut pruv_spawn_failed = false;")
        for i, call in enumerate(prepared):
            s.emit(d, "if !pruv_spawn_failed {")
            if s.par_hook == "spawn" and i == 1:
                s.emit(d + 1, "pruv_spawn_failed = true;")
            else:
                s.emit(d + 1, "match ::std::thread::Builder::new().spawn_scoped(pruv_scope, move || {")
                if hook:
                    s.emit(d + 2, "let mut gate = pruv_gate.0.lock().unwrap();")
                    s.emit(d + 2, "gate.1 += 1; pruv_gate.1.notify_all();")
                    s.emit(d + 2, "while !gate.0 || gate.1 < gate.2 { gate = pruv_gate.1.wait(gate).unwrap(); }")
                    s.emit(d + 2, "::std::mem::drop(gate);")
                    if s.par_hook == "panic" and i == 0:
                        s.emit(d + 2, 'panic!("pruv test worker panic");')
                s.emit(d + 2, call)
                s.emit(d + 1, "}) {")
                s.emit(d + 2, "::std::result::Result::Ok(h) => pruv_handles.push(h),")
                s.emit(d + 2, "::std::result::Result::Err(_) => pruv_spawn_failed = true,")
                s.emit(d + 1, "}")
            s.emit(d, "}")
        if hook:
            s.emit(d, "{ let mut gate = pruv_gate.0.lock().unwrap(); gate.0 = true; gate.2 = pruv_handles.len(); pruv_gate.1.notify_all(); }")
        s.emit(d, "let mut pruv_panicked = false;")
        s.emit(d, "for h in pruv_handles {")
        s.emit(d + 1, "if h.join().is_err() { pruv_panicked = true; }")
        if hook:
            s.emit(d + 1, 'eprintln!("pruv test joined");')
        s.emit(d, "}")
        s.emit(d, 'if pruv_spawn_failed { eprintln!("pruv: parallel spawn failed"); ::std::process::exit(101); }')
        s.emit(d, "if pruv_panicked { ::std::process::exit(101); }")
        s.emit(ind, "});")
        s.emit(ind - 1, "}")

    def mut_ref(s, name):
        """G5: a mutable borrow of the array `name`: a reborrow for an inout parameter (already `&mut [T]`)"""
        if name in s.inouts:
            return name if not s.cost and name in s.vt and not is_array(s.vt[name]) else "&mut *" + name
        return "&mut " + name

    def place_ref(s, p):
        """R-6g9-5: the `&mut` borrow of a place P ::= x | P.f | P[i] | if c {P} else {P}; an index is evaluated
        once before the borrow; a verified element is in bounds (index_oob proven at the binding)"""
        if isinstance(p, PlaceIf):
            return "if %s { %s } else { %s }" % (s.e(p.cond), s.place_ref(p.then), s.place_ref(p.els))
        if isinstance(p, Index):
            arr = s.place_path(p.array)
            if s.verified:
                return "{ let pruv_pi = (%s) as usize; unsafe { %s.get_unchecked_mut(pruv_pi) } }" % (s.e(p.index), arr)
            return "{ let pruv_pi = pruv_idx(%s, %s.len()); &mut %s[pruv_pi] }" % (s.e(p.index), arr, arr)
        return "&mut " + s.place_path(p) if not (isinstance(p, Var) and p.name in s.inouts) else "&mut *" + p.name

    def place_path(s, p):
        """the Rust lvalue of a field place rooted at a local or an inout parameter (auto-deref through `&mut`)"""
        root, path = field_place(p)
        if not path and root.name in s.inouts:
            return "(*%s)" % root.name
        return ".".join((root.name,) + path)

    def inout_ref(s, e, callee, i):
        root, path = field_place(e)
        if path:
            return "&mut " + ".".join((root.name,) + path)
        if root.name in s.resizable:
            if (callee, i) in s.array_resizes:
                return "&mut *" + root.name if s.cost else root.name
            return "&mut %s[..]" % root.name
        return s.mut_ref(root.name)

    def byte_bind(s, name, e):
        """LB6 SB (R-6g5-1): bind the source of a [u8] copy; a str is bound first (so a temporary String
        lives to the end of the block) and then shadowed by its bytes"""
        if e.ty == STR and not isinstance(e, StrLit):
            return "let %s = %s; let %s = %s.as_bytes(); " % (name, s.arr_ref(e), name, name)
        return "let %s = %s; " % (name, s.arr_ref(e))

    def arr_ref(s, e):
        """an array expression used as a borrowed slice; a string literal is borrowed as its `&'static str` (no alloc)"""
        if isinstance(e, StrLit):
            if e.empty_view:    # R-6g9-3: hand `&[]`
                return "&[]"
            if e.ty == ARRAY_U8:
                return "(%s).as_bytes()" % str_lit(e.value)
            if e.static_str or not s.cost:
                return str_lit(e.value)
        if e.borrowed_view:
            return s.e(e)
        if isinstance(e, Var) and e.name in s.refs:
            return e.name          # G9: a buffer payload view is already a reference
        if isinstance(e, Var) and e.name in s.inouts:
            return "&*" + e.name   # G5: a shared reborrow of an inout parameter
        if isinstance(e, Var) and (e.name not in s.params or e.name in s.take_arrays):
            return "&" + e.name
        if isinstance(e, Var):
            return e.name
        if isinstance(e, Call) and e.name == "subview":
            return s.e(e)
        if isinstance(e, Call) and e.name == "vec_get" and e.name not in s.fmap and e.ty == STR:
            return s.e(e)
        return "&" + s.e(e)

    def cref(s, x):
        """a collection expression in a borrow position (G6 section 12.1)"""
        if isinstance(x, Var):
            return x.name if x.name in s.borrowed or x.name in s.refs else "&" + x.name
        return "&(%s)" % s.e(x)

    def unchecked_catalog(s):
        return s.verified and s.fmode != "all" and not any(f.foreign is not None for f in s.prog.functions)

    def borrowed_map_key(s, key):
        if isinstance(key, StrLit):
            return str_lit(key.value)
        if key.ty.name == "string":
            return "%s.as_str()" % s.e(key)
        return s.arr_ref(key)

    def catalog_call(s, e):
        """executable catalog operation -> call of its private boundary helper"""
        from foreign import CATALOG
        cat = CATALOG[e.name]
        low = tuple(e.inst["args"]) if e.inst else ()
        src = tuple(e.inst.get("src_args", low)) if e.inst else ()
        if e.name in ("map_remove", "map_take"):
            s.fg.record_inline_catalog(e.name, src)
            place, key = e.args
            k = s.borrowed_map_key(key) if low[0].name == "string" else "&" + s.e(key)
            removed = "%s.remove(%s)" % (s.e(place.expr), k)
            return removed + ".is_some()" if e.name == "map_remove" else removed
        if e.name == "vec_reserve":
            s.fg.record_inline_catalog(e.name, src)
            place, cnt = s.e(e.args[0].expr), s.e(e.args[1])
            if s.unchecked_catalog():
                return "%s.reserve((%s) as usize)" % (place, cnt)
            return ('{ let pruv_n: i64 = %s; if pruv_n < 0 || pruv_n > (1i64 << 40) '
                    '{ pruv_g6_req("vec_reserve", "0 <= n && n <= 2^40"); } %s.reserve((pruv_n) as usize) }'
                    % (cnt, place))
        if e.name == "vec_clear":
            s.fg.record_inline_catalog(e.name, src)
            return "%s.clear()" % s.e(e.args[0].expr)
        if e.name == "vec_extend_from_slice":   # D95
            s.fg.record_inline_catalog(e.name, src)
            place, src_arr = s.e(e.args[0].expr), s.e(e.args[1])
            if s.unchecked_catalog():
                return "%s.extend_from_slice(&(%s)[..])" % (place, src_arr)
            return ('{ let pruv_s = &(%s)[..]; if ((%s.len() as u64) + (pruv_s.len() as u64)) > (1u64 << 40) '
                    '{ pruv_g6_req("vec_extend_from_slice", "len(v) + len(s) <= 2^40"); } %s.extend_from_slice(pruv_s) }'
                    % (src_arr, place, place))
        if e.name == "string_eq":
            s.fg.record_inline_catalog(e.name, src)
            operands = []
            for arg in e.args:
                if isinstance(arg, StrLit):
                    operands.append("%s.as_bytes()" % str_lit(arg.value))
                elif arg.ty == ARRAY_U8:
                    operands.append("&(%s)[..]" % s.e(arg))
                else:
                    operands.append("(%s).as_bytes()" % s.e(arg))
            return "(%s == %s)" % tuple(operands)
        # A bound proof can depend on an unchecked user foreign postcondition.
        if e.name == "vec_get" and e.ty == STR:
            s.fg.record_inline_catalog(e.name, src)
            v, i = map(s.e, e.args)
            if s.unchecked_catalog():
                return "(unsafe { (%s).get_unchecked((%s) as usize) }.as_str())" % (v, i)
            index = "<usize as ::std::convert::TryFrom<i64>>::try_from(%s).unwrap_or(usize::MAX)" % i
            if s.verified:
                return "((%s)[%s].as_str())" % (v, index)
            return ('((%s).get(%s).unwrap_or_else(|| pruv_g6_req("vec_get", "0 <= i && i < len(v)")).as_str())'
                    % (v, index))
        if s.rec_vec_get(e):
            return s.rec_vec_view(e)
        if e.name == "vec_get" and s.is_ext(low[-1]):
            s.fg.record_inline_catalog(e.name, src)
            v, i = map(s.e, e.args)
            if s.unchecked_catalog():
                return "(unsafe { %s.get_unchecked(%s as usize) })" % (v, i)
            return "(%s)[(%s) as usize]" % (v, i)
        if e.name == "vec_get" and s.unchecked_catalog():
            s.fg.record_inline_catalog(e.name, src)
            v, i = e.args
            return "(unsafe { *(%s).get_unchecked((%s) as usize) })" % (s.e(v), s.e(i))
        # vec_push onto a fresh vec_new: one element at exact capacity, the hand Rust `vec![x]` (P32L, R-6d-2)
        if (e.name == "vec_push" and isinstance(e.args[0], Call) and e.args[0].name == "vec_new"
                and s.unchecked_catalog()):
            s.fg.record_inline_catalog("vec_new", src[:1])
            s.fg.record_inline_catalog(e.name, src)
            return "::std::vec![%s]" % s.e(e.args[1])
        name = s.fg.catalog_helper(e.name, low, src)
        # string_to_str of an owned local that the enclosing return moves: the String itself is the copy.
        if getattr(e, "take_result_error", False) or (
                e.name == "string_to_str" and not s.cost and isinstance(e.args[0], Var) and e.args[0].name in s.moving):
            return s.e(e.args[0])
        args = []
        for i, (p, x) in enumerate(zip(cat.params, e.args)):
            if p.mode == "inout" and isinstance(x, InoutArg):
                args.append(s.inout_ref(x.expr, e.name, i))
            elif e.name == "map_get" and p.name == "k" and low[0].name == "string":
                args.append(s.borrowed_map_key(x))
            elif is_collection(p.type):
                args.append(s.e(x) if p.take else s.cref(x))
            elif p.type == STR or (e.name in ("file_write_bytes", "proc_write_bytes", "net_write_bytes", "ev_write", "ev_proc_write", "ev_proc_write_for") and p.type == ARRAY_U8):
                args.append(s.arr_ref(x))
            else:
                args.append(s.e(x))
        return "%s(%s)" % (name, ", ".join(args))

    def array_resize_expr(s, e, place):
        if e.name == "resize":
            return s.resize_expr(e, place)
        check = ("" if s.verified else
                 "if pruv_m < 0 || (pruv_m as u64) > (%s.len() as u64) { pruv_oob(pruv_m, %s.len()) } "
                 % (place, place))
        return "{ let pruv_m: i64 = %s; %s%s.truncate(pruv_m as usize); }" % (s.e(e.args[1]), check, place)

    def resize_expr(s, e, place=None):
        a = e.args
        bool_arr = is_bool_array(a[0].ty)
        vty = "bool" if bool_arr else "i64"
        if is_u8_array(a[0].ty):
            fill = "pruv_v as u8"
        elif bool_arr or not s.verified or s.layout(e) == "i64":
            fill = "pruv_v"
        else:
            fill = "pruv_v as %s" % s.layout(e)
        chk = ""
        if not s.verified:
            scratch = getattr(e, "resize_scratch", False)
            lower = "0" if scratch else "(pruv_a.len() as i64)"
            limit = "0" if scratch else "len"
            chk = ('if pruv_n < %s || pruv_n > (1i64 << 40) '
                   '{ pruv_ovf("resize length outside %s..2^40") } ' % (lower, limit))
            if is_u8_array(a[0].ty):
                chk += "if pruv_v < 0 || pruv_v > 255 { pruv_elem_oor(pruv_v) } "
        if place is not None:
            return ("{ let pruv_n: i64 = %s; let pruv_v: %s = %s; %s%s.reserve_exact((pruv_n as usize).saturating_sub(%s.len())); %s.resize(pruv_n as usize, %s); }"
                    % (s.e(a[1]), vty, s.e(a[2]), chk.replace("pruv_a.len()", place + ".len()"),
                       place, place, place, fill))
        # source order: a, n, v
        return ("{ let pruv_a = %s; let pruv_n: i64 = %s; let pruv_v: %s = %s; %s"
                "let mut pruv_t = ::std::vec::Vec::with_capacity(pruv_n as usize); pruv_t.extend_from_slice(pruv_a); "
                "pruv_t.resize(pruv_n as usize, %s); pruv_t }"
                % (s.arr_ref(a[0]), s.e(a[1]), vty, s.e(a[2]), chk, fill))

    def e(s, e):
        if isinstance(e, Var) and e.name in s.try_values:
            return s.try_values.pop(e.name)
        if isinstance(e, IntLit):
            if e.value == -2 ** 63:
                return "i64::MIN"
            return "(%di64)" % e.value if e.value < 0 else "%di64" % e.value
        if isinstance(e, BoolLit):
            return "true" if e.value else "false"
        if isinstance(e, Var):
            if (e.name in s.refs and e.name not in s.inouts and e.ty is not None and is_user_type(e.ty)
                    and not s.is_ext(e.ty)):
                return "(*%s)" % e.name
            if e.name in s.inouts and not (is_array(e.ty) or e.ty == STR or is_collection(e.ty) or s.is_ext(e.ty)):
                return "(*%s)" % e.name
            return e.name
        if isinstance(e, Move):   # G6: Rust moves
            if e.fields and e.name in s.owned_binders:   # LB9 (R-6g8-4): a consuming match binder owns its payload
                return "%s.%s" % (e.name, ".".join(e.fields))
            if e.fields:
                return "::std::mem::take(&mut %s.%s)" % (e.name, ".".join(e.fields))
            if e.name in s.resizable:
                return "::std::mem::take(%s)" % e.name
            return e.name
        if isinstance(e, Unary):
            if not s.verified and e.op == "-":
                return "pruv_neg(%s)" % s.e(e.operand)
            return "(%s%s)" % ("!" if e.op == "~" else e.op, s.e(e.operand))
        if isinstance(e, Binary):
            if not s.verified and e.op in SAFE_ARITH:
                return "%s(%s, %s)" % (SAFE_ARITH[e.op], s.e(e.left), s.e(e.right))
            if not s.verified and e.op in SHIFT_OPS:
                s.need_shift.add(e.op)
                return "pruv_%s_chk(%s, %s)" % (SHIFT_OPS[e.op], s.e(e.left), s.e(e.right))
            return "(%s %s %s)" % (s.e(e.left), e.op, s.e(e.right))
        if isinstance(e, StrLit):  # G2 B2
            return "::std::string::String::from(%s)" % str_lit(e.value)
        if isinstance(e, Index):
            if is_str(e.array.ty):  # G2 B2
                if not s.verified:
                    return "pruv_get_str(%s, %s)" % (s.arr_ref(e.array), s.e(e.index))
                return "((unsafe { *%s.as_bytes().get_unchecked((%s) as usize) }) as i64)" % (s.e(e.array), s.e(e.index))
            if is_bool_array(e.array.ty):  # G2 A5
                if not s.verified:
                    return "pruv_get_bool(%s, %s)" % (s.arr_ref(e.array), s.e(e.index))
                return "(unsafe { *%s.get_unchecked((%s) as usize) })" % (s.e(e.array), s.e(e.index))
            if is_u8_array(e.array.ty):
                if not s.verified:
                    return "pruv_get_u8(%s, %s)" % (s.arr_ref(e.array), s.e(e.index))
                return "((unsafe { *%s.get_unchecked((%s) as usize) }) as i64)" % (s.e(e.array), s.e(e.index))
            if not s.verified:
                return "pruv_get(%s, %s)" % (s.arr_ref(e.array), s.e(e.index))
            if s.layout(e.array) != "i64":
                return "((unsafe { *%s.get_unchecked((%s) as usize) }) as i64)" % (s.e(e.array), s.e(e.index))
            return "(unsafe { *%s.get_unchecked((%s) as usize) })" % (s.e(e.array), s.e(e.index))
        if isinstance(e, Ctor):
            if e.ty is not None and e.ty.args:   # G3: explicit payload type, never left to Rust inference
                rx = rty(e.ty.args[0])
                if e.name == "None":
                    return "None::<%s>" % rx
                view = e.name in ("Some", "Ok") and view_payload(e.ty) is not None
                arg = "()" if e.arg is None else s.arr_ref(e.arg) if view else s.e(e.arg)
                if e.name == "Some":
                    return "Some::<%s>(%s)" % (rx, arg)
                error = rty(result_error(e.ty))
                return "%s::<%s, %s>(%s)" % (e.name, rx, error, arg)
            return "None" if e.name == "None" else "%s(%s)" % (e.name, s.e(e.arg))
        if isinstance(e, StructLit):
            # declaration order = left-to-right evaluation; parenthesized so `if S { .. }.x` parses
            inits = {f.name: f.value for f in e.fields}
            fs = s.table.fields(Type(e.name))
            if not fs:
                return "(%s {})" % e.name
            if s.ext:   # G9: constructor fields are owning positions; a recursive edge is boxed
                return "(%s { %s })" % (e.name, ", ".join(
                    "%s: %s" % (f.name, s.arr_ref(inits[f.name]) if f.ref else
                                s.field_value(inits[f.name], "%s.%s" % (e.name, f.name))) for f in fs))
            return "(%s { %s })" % (e.name, ", ".join("%s: %s" % (f.name, s.e(inits[f.name])) for f in fs))
        if isinstance(e, EnumCtor):
            ctor = "%s::%s" % (e.enum_name, e.variant)
            if e.args and s.ext:   # G9
                vf = s.table.variant(Type(e.enum_name), e.variant).fields
                return "%s(%s)" % (ctor, ", ".join(
                    s.field_value(a, "%s::%s.%s" % (e.enum_name, e.variant, fd.name)) for a, fd in zip(e.args, vf)))
            return "%s(%s)" % (ctor, ", ".join(s.e(a) for a in e.args)) if e.args else ctor
        if isinstance(e, FieldAccess):
            b, path = e.base, [e.field]
            while isinstance(b, FieldAccess):   # D90: a nested field path stays one place inside the block
                path.insert(0, b.field)
                b = b.base
            if s.rec_vec_get(b):
                return "%s.%s" % (s.rec_vec_view(b), ".".join(path))
            if (isinstance(b, Call) and b.name == "vec_get" and b.name not in s.fmap and s.is_ext(b.ty)
                    and s.unchecked_catalog()):
                s.fg.record_inline_catalog(b.name, tuple(b.inst.get("src_args", b.inst["args"])))
                f = "(*(%s).get_unchecked((%s) as usize)).%s" % (s.e(b.args[0]), s.e(b.args[1]), ".".join(path))
                return "(unsafe { &%s })" % f if e.ty is not None and (is_collection(e.ty) or is_array(e.ty)) else "(unsafe { %s })" % f
            return "%s.%s" % (s.e(e.base), e.field)
        if isinstance(e, Call):
            n, a = e.name, e.args
            if s.cost and n in ("alloc", "copy", "truncate", "resize") and not getattr(e, "pruv_cost_wrapped", False):
                # Count the validated result length; nested argument calls still spend.
                import copy
                plain = copy.copy(e)
                plain.pruv_cost_wrapped = True
                result = s.e(plain)
                return '{ let pruv_buffer = %s; %s pruv_buffer }' % (result, s.cost_add('(pruv_buffer.len() as u128).checked_add(1).unwrap_or_else(|| panic!("pruv: cost exceeded"))'))
            if n == "len":
                return "(%s.len() as i64)" % s.e(a[0])
            if n == "alloc":
                if is_bool_array(e.ty):  # G2 A5
                    if not s.verified:
                        return "pruv_alloc_bool(%s, %s)" % (s.e(a[0]), s.e(a[1]))
                    return "{ let pruv_n: i64 = %s; let pruv_v: bool = %s; vec![pruv_v; pruv_n as usize] }" % (s.e(a[0]), s.e(a[1]))
                if is_u8_array(e.ty):
                    if not s.verified:
                        return "pruv_alloc_u8(%s, %s)" % (s.e(a[0]), s.e(a[1]))
                    return "{ let pruv_n: i64 = %s; let pruv_v: i64 = %s; vec![pruv_v as u8; pruv_n as usize] }" % (s.e(a[0]), s.e(a[1]))
                if not s.verified:
                    return "pruv_alloc(%s, %s)" % (s.e(a[0]), s.e(a[1]))
                if s.layout(e) != "i64":
                    return "{ let pruv_n: i64 = %s; let pruv_v: i64 = %s; vec![pruv_v as %s; pruv_n as usize] }" % (s.e(a[0]), s.e(a[1]), s.layout(e))
                # source order: n before v (vec![v; n] would evaluate v first)
                return "{ let pruv_n: i64 = %s; let pruv_v: i64 = %s; vec![pruv_v; pruv_n as usize] }" % (s.e(a[0]), s.e(a[1]))
            if n == "copy":
                # copy of an owned local that the enclosing return moves: the local itself is the copy.
                if not s.cost and isinstance(a[0], Var) and a[0].name in s.moving:
                    return s.e(a[0])
                if (not s.cost and not a[0].borrowed_view
                        and isinstance(a[0], FieldAccess) and isinstance(a[0].base, Var)
                        and a[0].base.name in s.moving and not s.field_boxed(a[0].base.ty, a[0].field)):
                    return s.e(a[0])
                if is_str(a[0].ty):  # G2 B2
                    return "%s.to_owned()" % s.e(a[0])
                if isinstance(a[0], Call) and a[0].name == "subview":   # LG8: copy(subview(a, lo, hi))
                    return "(%s).to_vec()" % s.e(a[0])
                return "%s.to_vec()" % s.e(a[0])
            if n == "truncate":
                # truncate of an owned local at its last use shortens the local in place; otherwise copy the prefix (R03L: a shortened
                # vector with over half its capacity unused releases the rest)
                if not s.cost and isinstance(a[0], Var) and a[0].name in s.moving:
                    chk = ("" if s.verified else "if pruv_m < 0 || (pruv_m as u64) > (pruv_t.len() as u64) "
                           "{ pruv_oob(pruv_m, pruv_t.len()) } ")
                    return ("{ let pruv_m: i64 = %s; let mut pruv_t = %s; %spruv_t.truncate(pruv_m as usize); "
                            "if pruv_t.capacity() / 2 > pruv_t.len() { pruv_t.shrink_to_fit(); } pruv_t }"
                            % (s.e(a[1]), s.e(a[0]), chk))
                if not s.verified:
                    return ("{ let pruv_a = %s; let pruv_m: i64 = %s; if pruv_m < 0 || (pruv_m as u64) > (pruv_a.len() as u64) "
                            "{ pruv_oob(pruv_m, pruv_a.len()) } pruv_a[..(pruv_m as usize)].to_vec() }" % (s.arr_ref(a[0]), s.e(a[1])))
                return ("{ let pruv_a = %s; let pruv_m: i64 = %s; unsafe { pruv_a.get_unchecked(..(pruv_m as usize)) }.to_vec() }"
                        % (s.arr_ref(a[0]), s.e(a[1])))
            if n == "resize":
                return s.resize_expr(e)
            if n in ARRAY_GROW_BUILTINS:
                dest = a[0].expr
                if n == "arr_append":
                    place = s.e(dest)
                    src, start, cnt = s.byte_bind("pruv_src", a[1]), s.e(a[2]), s.e(a[3])
                    head = ("{ %slet pruv_s: i64 = %s; let pruv_n: i64 = %s; " % (src, start, cnt))
                    if not s.verified:
                        head += ("if pruv_s < 0 || pruv_n < 0 || pruv_s > (pruv_src.len() as i64) - pruv_n "
                                 "{ pruv_oob(pruv_s, pruv_src.len()) } "
                                 'if pruv_n > (1i64 << 40) - (%s.len() as i64) { pruv_ovf("arr_append length exceeds 2^40") } ' % place)
                    span = "(pruv_s as usize)..((pruv_s + pruv_n) as usize)"
                    tail = ("unsafe { pruv_src.get_unchecked(%s) }" if s.verified else "&pruv_src[%s]") % span
                    if dest.ty == a[1].ty or a[1].ty == STR:   # LB6 SB: a str source appends its bytes
                        return "%s%s.extend_from_slice(%s) }" % (head, place, tail)
                    return "%s%s.extend((%s).iter().map(|&x| x as i64)) }" % (head, place, tail)
                place = s.e(dest)     # LB6 IF: a struct field place is pushed in place
                value = s.e(a[1]) if len(a) == 2 else None
                if n == "arr_reserve":
                    check = ("" if s.verified else
                             'if pruv_n < 0 || pruv_n > (1i64 << 40) { pruv_ovf("arr_reserve length outside 0..2^40") } ')
                    return "{ let pruv_n: i64 = %s; %s%s.reserve_exact((pruv_n) as usize) }" % (value, check, place)
                u8 = is_u8_array(dest.ty)
                vty = "bool" if is_bool_array(dest.ty) else "i64"
                layout = "u8" if u8 else s.layout(dest)
                elem = "pruv_v" if layout == "i64" or vty == "bool" else "pruv_v as " + layout
                if len(a) > 2:  # LB6 SX (R-6g5-5): k values in one step, from a stack array as hand Rust does
                    if s.verified:
                        vals = [s.e(v) if elem == "pruv_v" else "(%s) as %s" % (s.e(v), layout) for v in a[1:]]
                        return "%s.extend_from_slice(&[%s])" % (place, ", ".join(vals))
                    head = "".join("let pruv_v%d: %s = %s; " % (i, vty, s.e(v)) for i, v in enumerate(a[1:]))
                    check = 'if %s.len() > (1usize << 40) - %d { pruv_ovf("arr_push length exceeds 2^40") } ' % (place, len(a) - 1)
                    if u8:
                        check += "".join("if pruv_v%d < 0 || pruv_v%d > 255 { pruv_elem_oor(pruv_v%d) } " % (i, i, i)
                                         for i in range(len(a) - 1))
                    vals = [elem.replace("pruv_v", "pruv_v%d" % i) for i in range(len(a) - 1)]
                    return "{ %s%s%s.extend_from_slice(&[%s]) }" % (head, check, place, ", ".join(vals))
                check = ""
                if not s.verified:
                    check = 'if %s.len() >= (1usize << 40) { pruv_ovf("arr_push length exceeds 2^40") } ' % place
                    if u8:
                        check += "if pruv_v < 0 || pruv_v > 255 { pruv_elem_oor(pruv_v) } "
                return "{ let pruv_v: %s = %s; %s%s.push(%s) }" % (vty, value, check, place, elem)
            if n == "find_byte":  # LB6 MC (R-6g5-3): the first b in a[lo..hi], or hi; source order a, b, lo, hi
                head = ("{ %slet pruv_b: i64 = %s; let pruv_lo: i64 = %s; let pruv_hi: i64 = %s; "
                        % (s.byte_bind("pruv_fa", a[0]), s.e(a[1]), s.e(a[2]), s.e(a[3])))
                rng = "(pruv_lo as usize)..(pruv_hi as usize)"
                if s.verified:
                    sl = "unsafe { pruv_fa.get_unchecked(%s) }" % rng
                else:
                    head += ("if pruv_lo < 0 || pruv_lo > pruv_hi || pruv_hi > (pruv_fa.len() as i64) "
                             "{ pruv_oob(pruv_hi, pruv_fa.len()) } "
                             "if pruv_b < 0 || pruv_b > 255 { panic!(\"pruv: element out of range: {} (u8 0..=255)\", pruv_b) } ")
                    sl = "pruv_fa[%s]" % rng
                return ("%smatch %s.iter().position(|&pruv_x| pruv_x == pruv_b as u8) "
                        "{ Some(pruv_p) => pruv_lo + pruv_p as i64, None => pruv_hi } }" % (head, sl))
            if n == "eq_range":  # L3-10: eq_range: slice equality of a[i..i+n] and b[j..j+n]
                # the five arguments are bound once each, in source order a, i, b, j, n (SPEC.md eq_range)
                head = ("{ let pruv_ea = %s; let pruv_i: i64 = %s; let pruv_eb = %s; let pruv_j: i64 = %s; "
                        "let pruv_n: i64 = %s; " % (s.arr_ref(a[0]), s.e(a[1]), s.arr_ref(a[2]), s.e(a[3]), s.e(a[4])))
                ri = "(pruv_i as usize)..((pruv_i + pruv_n) as usize)"
                rj = "(pruv_j as usize)..((pruv_j + pruv_n) as usize)"
                if s.verified:
                    return ("%s(unsafe { pruv_ea.get_unchecked(%s) }) == (unsafe { pruv_eb.get_unchecked(%s) }) }"
                            % (head, ri, rj))
                # safe mode: the code-path out-of-range failure of plain indexing is pruv_oob (pruv_ud is contract-only)
                return ("%sif pruv_i < 0 || pruv_j < 0 || pruv_n < 0 || pruv_i > (pruv_ea.len() as i64) - pruv_n "
                        "|| pruv_j > (pruv_eb.len() as i64) - pruv_n { pruv_oob(pruv_i, pruv_ea.len()) } "
                        "pruv_ea[%s] == pruv_eb[%s] }" % (head, ri, rj))
            if n == "copy_within":
                name = a[0].expr.name
                head = ("{ let pruv_i: i64 = %s; let pruv_j: i64 = %s; let pruv_n: i64 = %s; "
                        % (s.e(a[1]), s.e(a[2]), s.e(a[3])))
                if s.verified:
                    return ("%sunsafe { let pa = %s.as_mut_ptr(); ::core::ptr::copy(pa.add((pruv_j) as usize), "
                            "pa.add((pruv_i) as usize), (pruv_n) as usize) }; }" % (head, name))
                return ("%sif pruv_i < 0 || pruv_n < 0 || pruv_i > (%s.len() as i64) - pruv_n { pruv_oob(pruv_i, %s.len()) } "
                        "if pruv_j < 0 || pruv_j > (%s.len() as i64) - pruv_n { pruv_oob(pruv_j, %s.len()) } "
                        "%s.copy_within((pruv_j) as usize..((pruv_j) + (pruv_n)) as usize, (pruv_i) as usize); }"
                        % (head, name, name, name, name, name))
            if n == "copy_range":  # L-copy-range: dst[i..i+n].copy_from_slice(&src[j..j+n]); i, src, j, n bound once in source order
                name = s.e(a[0].expr)
                if same_place(a[0].expr, a[2]):
                    head = ("{ let pruv_i: i64 = %s; let pruv_j: i64 = %s; let pruv_n: i64 = %s; "
                            % (s.e(a[1]), s.e(a[3]), s.e(a[4])))
                    if not s.verified:
                        head += ("if pruv_i < 0 || pruv_n < 0 || pruv_i > (%s.len() as i64) - pruv_n { pruv_oob(pruv_i, %s.len()) } "
                                 "if pruv_j < 0 || pruv_j > (%s.len() as i64) - pruv_n { pruv_oob(pruv_j, %s.len()) } "
                                 % (name, name, name, name))
                    return ("%s%s.copy_within((pruv_j as usize)..((pruv_j + pruv_n) as usize), pruv_i as usize); }"
                            % (head, name))
                head = ("{ let pruv_i: i64 = %s; %slet pruv_j: i64 = %s; let pruv_n: i64 = %s; "
                        % (s.e(a[1]), s.byte_bind("pruv_cs", a[2]), s.e(a[3]), s.e(a[4])))
                ri = "(pruv_i as usize)..((pruv_i + pruv_n) as usize)"
                rj = "(pruv_j as usize)..((pruv_j + pruv_n) as usize)"
                if s.verified:
                    return ("%sunsafe { %s.get_unchecked_mut(%s) }.copy_from_slice(unsafe { pruv_cs.get_unchecked(%s) }); }"
                            % (head, name, ri, rj))
                # P13i2a.audit-1: a bad destination range reports (i, len(dst)), a bad source range (j, len(src))
                return ("%sif pruv_i < 0 || pruv_n < 0 || pruv_i > (%s.len() as i64) - pruv_n { pruv_oob(pruv_i, %s.len()) } "
                        "if pruv_j < 0 || pruv_j > (pruv_cs.len() as i64) - pruv_n { pruv_oob(pruv_j, pruv_cs.len()) } "
                        "%s[%s].copy_from_slice(&pruv_cs[%s]); }" % (head, name, name, name, ri, rj))
            if n in ("load_be", "load_le"):
                order = n[-2:]
                nb = a[2].value
                chk = "" if s.verified else ("if pruv_i < 0 || (pruv_i as u64) + %d > (pruv_a.len() as u64) "
                                             "{ pruv_oob(pruv_i, pruv_a.len()) } " % nb)
                rng = "(pruv_i as usize)..((pruv_i as usize) + %d)" % nb
                sl = "unsafe { pruv_a.get_unchecked(%s) }" % rng if s.verified else "&pruv_a[%s]" % rng
                if nb == 8:
                    word = ("i64::from_%s_bytes(<[u8; 8] as ::std::convert::TryFrom<&[u8]>>::try_from(%s).unwrap())" % (order, sl))
                    tail = word
                else:
                    byte_range = "..%d" % nb if order == "le" else "%d.." % (8 - nb)
                    tail = ("let mut pruv_w = [0u8; 8]; pruv_w[%s].copy_from_slice(%s); u64::from_%s_bytes(pruv_w) as i64"
                            % (byte_range, sl, order))
                return "{ let pruv_a: &[u8] = %s; let pruv_i: i64 = %s; %s%s }" % (s.arr_ref(a[0]), s.e(a[1]), chk, tail)
            if n in ("store_be", "store_le"):
                order = n[-2:]
                nb = a[3].value
                name = a[0].expr.name if isinstance(a[0].expr, Var) else s.e(a[0].expr)   # G-DB31-1: a field place
                chk = "" if s.verified else ("if pruv_i < 0 || (pruv_i as u64) + %d > (%s.len() as u64) "
                                             "{ pruv_oob(pruv_i, %s.len()) } " % (nb, name, name))
                rng = "(pruv_i as usize)..((pruv_i as usize) + %d)" % nb
                dst = "unsafe { %s.get_unchecked_mut(%s) }" % (name, rng) if s.verified else "%s[%s]" % (name, rng)
                byte_range = "..%d" % nb if order == "le" else "%d.." % (8 - nb)
                return ("{ let pruv_i: i64 = %s; let pruv_w: i64 = %s; %s%s.copy_from_slice(&(pruv_w as u64).to_%s_bytes()[%s]); }"
                        % (s.e(a[1]), s.e(a[2]), chk, dst, order, byte_range))
            if n == "print":
                s.printed = True
                return 'println!("{}", %s)' % s.e(a[0])
            if n == "is_char_boundary":  # G2 B2
                return "pruv_str_boundary(%s, %s)" % (s.arr_ref(a[0]), s.e(a[1]))
            if n in SHIFT_SAFE:  # P2 batch 2: --no-verify checks the count in a helper; verified relies on shift_range
                if not s.verified:
                    s.need_shift.add(n)
                    return "%s(%s, %s)" % (SHIFT_SAFE[n], s.e(a[0]), s.e(a[1]))
                return "((((%s) as u64) %s ((%s) as u32)) as i64)" % (s.e(a[0]), BIT_RUST[n], s.e(a[1]))
            if n in WRAP_RUST:  # P2 batch 3: wraps by definition, same text with and without --no-verify
                return "(%s).%s(%s)" % (s.e(a[0]), n, s.e(a[1]))
            if n in BIT_RUST:  # P2: no overflow possible, same text with and without --no-verify
                return "(%s %s %s)" % (s.e(a[0]), BIT_RUST[n], s.e(a[1]))
            if n == "utf8_valid":
                return "::core::str::from_utf8(%s).is_ok()" % s.arr_ref(a[0])
            if n == "str_from_utf8":
                if not s.cost and isinstance(a[0], Var) and a[0].name in s.moving:
                    # LG33 (G-DB54-2): the owned local at its last use (last_copy) becomes the String's buffer
                    if s.verified or e is s.utf8_ok:
                        return "{ let pruv_v: ::std::vec::Vec<u8> = %s; unsafe { ::std::string::String::from_utf8_unchecked(pruv_v) } }" % s.e(a[0])
                    return ('{ let pruv_v: ::std::vec::Vec<u8> = %s; match ::std::string::String::from_utf8(pruv_v) { '
                            'Ok(pruv_s) => pruv_s, '
                            'Err(pruv_e) => panic!("pruv: invalid UTF-8 (len {})", pruv_e.as_bytes().len()), } }') % s.e(a[0])
                b = s.arr_ref(a[0])
                if s.verified or e is s.utf8_ok:
                    return "{ let pruv_b: &[u8] = %s; unsafe { ::std::string::String::from_utf8_unchecked(pruv_b.to_vec()) } }" % b
                return ('{ let pruv_b: &[u8] = %s; match ::core::str::from_utf8(pruv_b) { '
                        'Ok(pruv_s) => ::std::string::String::from(pruv_s), '
                        'Err(_) => panic!("pruv: invalid UTF-8 (len {})", pruv_b.len()), } }') % b
            if n == "subview":
                b, lo, hi = s.arr_ref(a[0]), s.e(a[1]), s.e(a[2])
                if e.ty == STR:   # LG25 (R-6g-10): a borrowed str view for a str parameter
                    from_str = a[0].ty == STR
                    if from_str and s.verified:
                        return "unsafe { (%s).get_unchecked(((%s) as usize)..((%s) as usize)) }" % (b, lo, hi)
                    if from_str:
                        return ("match (%s, %s, %s) { (pruv_a, pruv_lo, pruv_hi) => { "
                                "if !(pruv_lo >= 0 && pruv_lo <= pruv_hi && (pruv_hi as u64) <= (pruv_a.len() as u64)) "
                                "{ pruv_slice_oob(pruv_lo, pruv_hi, pruv_a.len()) } "
                                "if !(pruv_a.is_char_boundary(pruv_lo as usize) && pruv_a.is_char_boundary(pruv_hi as usize)) "
                                "{ pruv_slice_cut(pruv_lo, pruv_hi, pruv_a.len()) } "
                                "&pruv_a[(pruv_lo as usize)..(pruv_hi as usize)] } }" % (b, lo, hi))
                    if s.verified:
                        return ("unsafe { ::core::str::from_utf8_unchecked((%s).get_unchecked(((%s) as usize)..((%s) as usize))) }"
                                % (b, lo, hi))
                    return ("match (%s, %s, %s) { (pruv_a, pruv_lo, pruv_hi) => { "
                            "if !(pruv_lo >= 0 && pruv_lo <= pruv_hi && (pruv_hi as u64) <= (pruv_a.len() as u64)) "
                            "{ pruv_oob(pruv_lo, pruv_a.len()) } "
                            "let pruv_span: &[u8] = &pruv_a[(pruv_lo as usize)..(pruv_hi as usize)]; "
                            "match ::core::str::from_utf8(pruv_span) { Ok(pruv_t) => pruv_t, "
                            "Err(_) => panic!(\"pruv: invalid UTF-8 (len {})\", pruv_span.len()), } } }" % (b, lo, hi))
                if not s.verified:
                    return ("match (%s, %s, %s) { (pruv_a, pruv_lo, pruv_hi) => { "
                            "if !(pruv_lo >= 0 && pruv_lo <= pruv_hi && (pruv_hi as u64) <= (pruv_a.len() as u64)) "
                            "{ pruv_oob(pruv_lo, pruv_a.len()) } "
                            "&pruv_a[(pruv_lo as usize)..(pruv_hi as usize)] } }" % (b, lo, hi))
                return "unsafe { (%s).get_unchecked(((%s) as usize)..((%s) as usize)) }" % (b, lo, hi)
            if n == "substr":  # G2 B2
                if not s.verified:
                    return "pruv_substr(%s, %s, %s)" % (s.arr_ref(a[0]), s.e(a[1]), s.e(a[2]))
                return ("{ let pruv_s: &str = %s; let pruv_lo: i64 = %s; let pruv_hi: i64 = %s; "
                        "::std::string::String::from(unsafe { pruv_s.get_unchecked((pruv_lo as usize)..(pruv_hi as usize)) }) }") % (
                            s.arr_ref(a[0]), s.e(a[1]), s.e(a[2]))
            if s.g6 and n in CATALOG and n not in s.fmap:
                return s.catalog_call(e)
            callee = s.fmap.get(n) if s.g6 else None
            if callee is not None and callee.foreign is not None:
                s.fg.called.add(n)
                s.fg.trusted_helper(callee)   # emitted on first use; the helper checks the selected requires itself
            args = []
            for i, x in enumerate(a):
                if isinstance(x, InoutArg):   # G5: mutable borrow / reborrow
                    args.append(s.inout_ref(x.expr, n, i))
                elif n in s.struct_inouts and isinstance(x, IntLit) and x.value >= 0:
                    args.append(str(x.value))
                elif isinstance(x, Move) and is_array(x.ty):
                    args.append(s.e(x))
                elif callee is not None and callee.params[i].type == STR and x.ty is not None and x.ty.name == "string":
                    args.append(s.borrowed_map_key(x))
                elif callee is not None and is_vec_slice_arg(x.ty, callee.params[i].type):
                    args.append(s.e(x) + ".as_slice()")
                elif s.g6 and (isinstance(x, Move) or (x.ty is not None and is_owned_value(x.ty))):
                    take = isinstance(x, Move) or (callee is not None and callee.params[i].take)
                    args.append(s.e(x) if take else s.cref(x))
                elif s.ext and s.is_ext(x.ty) and not (callee is not None and callee.params[i].take):
                    args.append(s.view(x))   # G9: an extended argument is a read view; a fresh temporary lives through the call
                elif x.ty is not None and (is_array(x.ty) or is_str(x.ty)):
                    args.append(s.arr_ref(x))
                else:
                    args.append(s.e(x))
            captured = []
            if s.cost and n in s.fmap:
                for i, arg in enumerate(args):
                    captured.append("let pruv_cost_arg_%d = %s;" % (i, arg))
                args = ["pruv_cost_arg_%d" % i for i in range(len(args))]
                captured.append(s.cost_call(n, args))
            if e is s.small_call:
                return "%s__pruv_small(&mut pruv_sb_%s%s)" % (n, s.small_buf, "".join(", " + x for x in args))
            if s.explicit:
                n = s.callee(n)
            elif s.verified and n in s.body_named:
                n = n + "__pruv_body"   # caller proved requires (T0: or passes the length guard)
            call = "%s(%s)" % (n, ", ".join(args))
            return "{ %s %s }" % (" ".join(captured), call) if captured else call
        raise Exception("codegen: contract-only expression in code: %r" % e)


RAW_STORE = """\
    /// The store was poisoned by a panic inside `PruvStore::with`; its state is never observed or reused.
    #[derive(Debug)]
    pub struct PruvPoisoned {}
    /// Owns resident state for the raw entries. Every call runs through `with`: the poison flag is set before the
    /// call and cleared only when it returns normally, so a caught panic leaves the store poisoned and every later
    /// `with` refuses without touching the state. Dropping a poisoned store only releases its buffers.
    /// Two rules the types cannot enforce are the adapter's: `T` owns the state (no reference or shared handle to it
    /// outside the store), and the closure given to `with` never catches a panic (no `catch_unwind` inside it).
    pub struct PruvStore<T> {
        data: T,
        poisoned: bool,
    }
    impl<T> PruvStore<T> {
        #[inline(always)]
        pub fn new(data: T) -> Self {
            PruvStore { data, poisoned: false }
        }
        #[inline(always)]
        pub fn is_poisoned(&self) -> bool {
            self.poisoned
        }
        #[inline(always)]
        pub fn with<R>(&mut self, f: impl FnOnce(&mut T) -> R) -> Result<R, PruvPoisoned> {
            if self.poisoned {
                return Err(PruvPoisoned {});
            }
            self.poisoned = true;
            let r = f(&mut self.data);
            self.poisoned = false;
            Ok(r)
        }
    }"""

# P13a.10: the store of a program whose entries cannot unwind (raw_unwind_free). Same API, no poison flag: a verified
# entry either panics in its entry length or requires checks, before it writes anything, or returns normally, so the state in the
# store is always the result of complete entry calls.
RAW_STORE_FREE = """\
    /// Never produced for this program: its entries cannot unwind (kept so adapters written against the poisoning
    /// store compile unchanged).
    #[derive(Debug)]
    pub struct PruvPoisoned {}
    /// Owns resident state for the raw entries. Every call runs through `with`. This program was verified and calls
    /// no foreign or runtime code, so an entry either panics in its entry length or `requires` checks, before it
    /// writes, or returns normally: the state is always the result of complete entry calls and the store has no poison flag
    /// (`with` always runs the closure, `is_poisoned` is always false). Two rules the types cannot enforce are the
    /// adapter's: `T` owns the state (no reference or shared handle to it outside the store), and the closure changes
    /// `T` only through the entries (a host write followed by a host panic is not caught here).
    pub struct PruvStore<T> {
        data: T,
    }
    impl<T> PruvStore<T> {
        #[inline(always)]
        pub fn new(data: T) -> Self {
            PruvStore { data }
        }
        #[inline(always)]
        pub fn is_poisoned(&self) -> bool {
            false
        }
        #[inline(always)]
        pub fn with<R>(&mut self, f: impl FnOnce(&mut T) -> R) -> Result<R, PruvPoisoned> {
            Ok(f(&mut self.data))
        }
    }"""

RAW_FORWARD = """\
        /// Raw entry `%s` (manifest raw_entries = "unsafe").
        ///
        /// # Safety
        ///
        /// The caller upholds every precondition of the entry: its `requires` (checked at run time unless the
        /// program was built with --trust-callers; array and str lengths above 2^40 are always rejected), and every array that carries resident state was produced by this
        /// program's entries and was not changed by other code since. A state that was being changed when a panic
        /// unwound must never be passed again; keep it in a `PruvStore`, which refuses reuse after a panic.
        #[inline(always)]
        pub unsafe fn %s(%s)%s {
            super::super::%s(%s)
        }"""


def raw_module(mode, items, unwind_free=False, adapter_len=False):
    """P09: the pruv_raw module of a program with manifest raw_entries = mode ("crate" or "unsafe"), without a
    trailing newline. items: (name, params text, return text, parameter names) of the host entries' public items in emission order.
    unwind_free (P13a.10, raw_unwind_free): the entries cannot unwind, so the store carries no poison flag.
    adapter_len (T0): the Rust backend under manifest "crate" (not "c"): the entries have no length check and the
    header states the adapter's length obligation."""
    store = RAW_STORE_FREE if unwind_free else RAW_STORE
    if mode == "crate":
        head = ['// P09: manifest raw_entries = "crate": the host entries above are pub(crate); a host adapter in this crate keeps',
                ("// resident state in pruv_raw::PruvStore; the entries cannot unwind, so the store has no poison flag."
                 if unwind_free else "// resident state in pruv_raw::PruvStore, which refuses every reuse after a panic.")]
        if adapter_len:
            head += ["// T0: these entries do not check array / str lengths. The adapter passes only arrays whose element count, and str", "// values whose byte length, is in 0..=2^40; breaking that is undefined behavior (SPEC section 7, host length bound)."]
        head += ["#[allow(dead_code)]", "pub mod pruv_raw {"]
        return "\n".join(head + [store, "}"])
    head = ['// P09: manifest raw_entries = "unsafe": the host entries above are pub(crate) and exported only as the documented',
            ("// pub unsafe fn forwarders in pruv_raw; resident state lives in pruv_raw::PruvStore (no poison flag: no unwind)."
             if unwind_free else
             "// pub unsafe fn forwarders in pruv_raw; resident state lives in pruv_raw::PruvStore, which refuses reuse after a panic."),
            "#[allow(dead_code)]", "pub mod pruv_raw {"]
    # the forwarders sit in a private child module so the program's own type names (which the glob import brings in)
    # never meet PruvStore, PruvPoisoned or the prelude names the store uses
    mid = ["    #[allow(unused_imports)]", "    pub use self::entries::*;", "    mod entries {",
           "        #[allow(unused_imports)]", "        use super::super::*;"]
    fwd = [RAW_FORWARD % (n, n, ps, ret, n, ", ".join(names)) for n, ps, ret, names in items]
    return "\n".join(head + [store] + mid + fwd + ["    }", "}"])


PROVEN_NAMES = ("PruvArr", "PruvStr", "pruv_arr_slow", "pruv_str_slow", "pruv_proven")

# The child module keeps the fields private even when the generated file is included at the host's crate root.
PROVEN_ARR = """\
mod pruv_proven_types {
    /// P18b (--proven-entries): a slice whose length was checked against 2^40 by `new` or `new_mut`.
    #[repr(transparent)]
    pub struct PruvArr<T>([T]);
    #[cold]
    #[inline(never)]
    fn pruv_arr_slow(n: usize) -> ! { panic!("pruv: invalid representation: {} (array length)", n) }
    #[allow(dead_code)]
    impl<T> PruvArr<T> {
        // The transparent casts preserve the slice metadata and the original borrow.
        #[inline]
        pub fn new(a: &[T]) -> &PruvArr<T> { if (a.len() as u64) > (1u64 << 40) { pruv_arr_slow(a.len()) } unsafe { &*(a as *const [T] as *const PruvArr<T>) } }
        #[inline]
        pub fn new_mut(a: &mut [T]) -> &mut PruvArr<T> { if (a.len() as u64) > (1u64 << 40) { pruv_arr_slow(a.len()) } unsafe { &mut *(a as *mut [T] as *mut PruvArr<T>) } }
        // The private field and the checked constructors establish the bound for both accessors.
        #[inline(always)]
        pub fn get(&self) -> &[T] { unsafe { ::std::hint::assert_unchecked((self.0.len() as u64) <= (1u64 << 40)) }; &self.0 }
        #[inline(always)]
        pub fn get_mut(&mut self) -> &mut [T] { unsafe { ::std::hint::assert_unchecked((self.0.len() as u64) <= (1u64 << 40)) }; &mut self.0 }
    }
"""

PROVEN_STR = """\
    /// P18b (--proven-entries): a string whose byte length was checked against 2^40 by `new`.
    #[repr(transparent)]
    pub struct PruvStr(str);
    #[cold]
    #[inline(never)]
    fn pruv_str_slow(n: usize) -> ! { panic!("pruv: invalid representation: {} (str length)", n) }
    #[allow(dead_code)]
    impl PruvStr {
        #[inline]
        pub fn new(s: &str) -> &PruvStr { if (s.len() as u64) > (1u64 << 40) { pruv_str_slow(s.len()) } unsafe { &*(s as *const str as *const PruvStr) } }
        #[inline(always)]
        pub fn get(&self) -> &str { unsafe { ::std::hint::assert_unchecked((self.0.len() as u64) <= (1u64 << 40)) }; &self.0 }
    }
"""


def proven_entry(f, param_types, vis, name, ret):
    checked = {p.name for p in len_params(f)}
    params, rebinds = [], []
    has_str = False
    for p, ty in zip(f.params, param_types):
        mapped = ty
        if p.name in checked:
            accessor = "get"
            if ty == "&str":
                mapped = "&PruvStr"
                has_str = True
            elif ty.startswith("&mut [") and ty.endswith("]"):
                mapped = "&mut PruvArr<%s>" % ty[6:-1]
                accessor = "get_mut"
            elif ty.startswith("&[") and ty.endswith("]"):
                mapped = "&PruvArr<%s>" % ty[2:-1]
            else:
                # Owned length-checked parameters need ownership transfer, so skip this entry.
                return None
            rebinds.append("    let %s: %s = %s.%s();" % (p.name, ty, p.name, accessor))
        params.append("%s: %s" % (p.name, mapped))
    val = None
    if f.params and f.params[-1].mode == "inout" and param_types[-1] == "&mut [i64]":
        rt = ret[4:] if ret else "()"
        args = [p.name for p in f.params[:-1]] + ["PruvArr::new_mut(&mut pruv_res)"]
        val = [
            "#[inline(always)]",
            f"{vis}fn {name}<const PRUV__K: usize>({', '.join(params[:-1])}) -> ({rt}, [i64; PRUV__K]) {{",
            '    const { assert!((PRUV__K as u64) <= (1u64 << 40), "pruv: by-value result longer than 2^40") };',
            "    let mut pruv_res = [0i64; PRUV__K];",
            f"    let pruv_r = super::{name}({', '.join(args)});",
            "    (pruv_r, pruv_res)",
            "}",
        ]
    return "%sfn %s(%s)%s {" % (vis, name, ", ".join(params), ret), rebinds, has_str, val


def proven_module(text, items):
    lines = text.split("\n")
    # The policy pass has already normalized explicit inline directives.
    attrs = {line: [attr] for attr, line in zip(lines, lines[1:]) if attr in ("#[inline]", "#[inline(always)]")}
    # R-7-3: a `#[cold]` fn is emitted as `#[cold]` then `#[inline(never)]`; its proven entry keeps both.
    attrs.update({line: [cold, attr] for cold, attr, line in zip(lines, lines[1:], lines[2:])
                  if cold == "#[cold]" and attr == "#[inline(never)]"})
    has_str = any(has_str for _, _, _, _, has_str, _ in items)
    prelude = PROVEN_ARR + (PROVEN_STR if has_str else "") + "}\n"
    exports = ["#[allow(unused_imports)]", "pub use self::pruv_proven_types::PruvArr;"]
    if has_str:
        exports.append("pub use self::pruv_proven_types::PruvStr;")
    module = ["", "/// P18b (--proven-entries): entries with lengths checked once by their parameter constructors.",
              "pub mod pruv_proven {", "    use super::*;"]
    vals = []
    for header, allow, signature, body, _, val in items:
        module.extend("    " + line for line in (allow, *attrs[header], signature))
        module.extend("    " + line for item in body for line in item.split("\n"))
        module.append("    }")
        if val is not None:
            vals.extend("        " + line for line in [allow, *val])
    if vals:
        module.extend(['    /// P18c (--proven-entries): by-value results, see SPEC.md "By-value results".',
                       "    pub mod val {", "        #[allow(unused_imports)]", "        use super::super::*;",
                       *vals, "    }"])
    return "\n" + prelude + "\n".join(exports + module + ["}", ""])


def raw_unwind_free(g):
    """P13a.10: no host entry of the program can unwind after it starts writing. Verified explicit mode erases every
    check inside bodies (Pruv callers call f__pruv_body, the caller proved requires; no cost counters) and every
    operation the verifier accepted is total on the proved domain (get_unchecked, plain arithmetic, lengths <= 2^40),
    so the only panics left are the entry's own length and requires checks, which run before the body. Foreign code (G6 boundary
    checks, trusted Rust), print (println! panics when stdout fails), the G10 file / process / socket / event runtimes
    and par groups (threads, locks) keep the poisoning store."""
    return bool(g.verified and not g.printed and not g.g6 and not g.g10 and not g.proc and not g.net and not g.ev and not g.par_callees)


def gen_rust(prog, verified=True, trust_callers=False, budget=1 << 30, plan=None, foreign_checks="requires",
             externs=(), proven_entries=False):
    """plan: a mono.SpecPlan for a generic program (computed here when omitted; PruvError specialization_limit
    propagates). A program without generics is generated directly from prog, exactly as before G3.
    foreign_checks: G6 boundary checks requires|all|off (docs/G6_SPEC.md section 12.3)."""
    return gen_rust_ex(prog, verified, trust_callers, budget, plan, foreign_checks, externs, proven_entries=proven_entries)[0]


def gen_rust_ex(prog, verified=True, trust_callers=False, budget=1 << 30, plan=None, foreign_checks="requires",
                externs=(), trust_out=None, proven_entries=False):
    """(rust text, manifest text or None). The manifest (G6_SPEC 12.4) exists only for programs that use G6;
    externs: [(name, path)] as given with --rust-extern.
    trust_out: G4b-2, a dict filled with the trust data (the concrete program and the ForeignGen or None)."""
    from typeinfo import g2_unimplemented_error, first_g3_use
    if foreign_checks not in ("requires", "all", "off"):
        raise ValueError("foreign_checks must be requires, all or off")
    if getattr(prog, "raw_entries", None) == "c":
        ext, _ = g9_sets(prog)
        for f in prog.functions:
            if not f.host_entry:
                continue
            if any(p.mode == "inout" and not is_array(p.type) for p in f.params):
                raise PruvError("codegen", f.line, f.col, "unsupported_type",
                                "host entry %s takes an inout struct, which has no C ABI" % f.name,
                                "use Rust raw_entries crate or unsafe", file=getattr(f, "file", None))
            if any(is_collection(p.type) or p.type.name in ext for p in f.params):
                raise PruvError("codegen", f.line, f.col, "unsupported_type",
                                "host entry %s takes a collection-bearing parameter, which has no C ABI" % f.name,
                                "use Rust raw_entries crate or unsafe", file=getattr(f, "file", None))
    from check import opaque_inline
    opaque_inline(prog)     # P13o: contract applications of opaque functions become their bodies
    if plan is None and (first_g3_use(prog) is not None or prog.explicit):   # G4 A2: explicit mode always names via mono
        from mono import specialize
        plan = specialize(prog)
    if plan is not None:
        prog = plan.program   # concrete: stems for specializations, no type parameters
    g2err = g2_unimplemented_error(prog, "codegen", skip=("array_u8", "array_bool", "str"))  # G2 B2
    if g2err is not None:  # same as the Rust generator's Err(String); drivers reject earlier with the diagnostic
        raise RuntimeError("unsupported_type: %s" % g2err.message)
    if proven_entries:
        for item in (*prog.types, *prog.functions):
            if item.name in PROVEN_NAMES:
                raise PruvError("codegen", 0, 0, "proven_name_clash",
                                "--proven-entries conflicts with Rust name " + item.name,
                                "rename the item or omit --proven-entries")
    from storage import plan_storage
    from stack import plan_stack
    storage = plan_storage(prog, verified)
    g = Gen(prog, verified, trust_callers, budget, foreign_checks, storage, plan_stack(prog, storage, verified))
    g.proven_entries = proven_entries
    if proven_entries:
        g.plan_fp()
    g.prelude()
    mark = len(g.out)
    for f in prog.functions:
        g.fn(f)
    # LB5: inserted first so every helper below goes in front of it (same order as pruv-core codegen.rs)
    if g.need_cidx_u8 and not g.has_cidx_u8:
        g.out.insert(mark, CONTRACT_PRELUDE_U8)
    # G9: size / equality helpers discovered while the requires checks were generated go in front of the functions
    # (declaration order, no addresses or hashes); nothing at all is added when no runtime requires needs them
    g.out[mark:mark] = g.g9_helper_texts()
    # P2: contract helpers of the bit builtins the requires checks used, in table order
    g.out[mark:mark] = [CONTRACT_SHIFT_HELPER % (n, n, n, BIT_RUST[n]) if n in SHIFT_SAFE
                        else CONTRACT_WRAP_HELPER % (n, n, n) if n in WRAP_RUST
                        else CONTRACT_BIT_HELPER % (n, n, BIT_RUST[n]) for n in BIT_RUST if n in g.need_bits]
    # P2 batch 2: --no-verify shift helpers used by the code, in table order
    g.out[mark:mark] = [SAFE_SHIFT_HELPER % (SHIFT_SAFE[n], BIT_RUST[n]) for n in SHIFT_SAFE if n in g.need_shift]
    g.out[mark:mark] = [SHIFT_OP_HELPER % (name, SHIFT_OP_BODY[op])
                        for op, name in SHIFT_OPS.items() if op in g.need_shift]
    g.out[mark:mark] = [contract_op_helper(op) for op in CONTRACT_OPS if op in g.need_bits]
    # P2 batch 4: the load_be contract helper, when a runtime contract used it
    if g.need_load_le:
        g.out[mark:mark] = [CONTRACT_LOAD_LE_HELPER]
    if g.need_load_be:
        g.out[mark:mark] = [CONTRACT_LOAD_HELPER]
    manifest = None
    if g.g6:
        # helpers are discovered while the functions are generated; they go in front of them, in first-use order
        g.out[mark:mark] = g.fg.helper_texts()
        manifest = g.fg.manifest(externs)
    if trust_out is not None:
        trust_out["program"] = prog
        trust_out["fg"] = g.fg if g.g6 else None
    text = inline_policy("\n".join(g.out) + "\n")
    if g.raw:   # P09: after inline_policy, so the module never changes an inlining decision
        text += raw_module(g.raw, g.raw_items, raw_unwind_free(g), g.raw_crate) + "\n"
    if g.proven_items:
        text += proven_module(text, g.proven_items)
    text = hide_bodies(text)
    return text, manifest


# A host macro_rules! defined before the generated file is in textual scope inside it and would replace
# the std macro the checks use (a host `panic!` that returns lets a violated requires run the body).
# Local forwarders shadow any host definition; the list is every macro the generator calls unqualified.
# Forward through the extern prelude: core:: for macros core has (no_std hosts keep compiling); no leading ::
# because in edition 2015 that names the crate root.
HIDE_MACROS = (("assert", "core"), ("eprintln", "std"), ("format_args", "core"), ("matches", "core"),
               ("panic", "core"), ("println", "std"), ("unreachable", "core"), ("vec", "std"))
HIDE_HEAD = "mod pruv_impl {\n" + "".join(
    "#[allow(unused_macros)]\nmacro_rules! %s { ($($t:tt)*) => { %s::%s!($($t)*) } }\n" % (m, c, m)
    for m, c in HIDE_MACROS)


def hide_bodies(text):
    # Hosts reach only re-exported pub items. Even a crate-root include! cannot name
    # unchecked __pruv_body fns or other private items. Keep lines unindented for
    # R-7-3 and line-anchored checks. Module boundaries do not change inlining.
    return HIDE_HEAD + text + "}\n#[allow(unused_imports)]\npub use self::pruv_impl::*;\n"


ASYNC_PRELUDE = r"""#[derive(Clone, Default)]
struct PruvEvPublic { kind: i64, open: bool, eof: bool, failed: bool, ends: [bool;2] }
#[derive(Default)]
struct PruvEvQueue { bytes: [::std::collections::VecDeque<u8>;2], ended: [bool;2], err: i64, closing: bool, accepting: bool, reported: bool, pending: ::std::collections::VecDeque<::std::net::TcpStream> }
struct PruvEvShared { state: ::std::sync::Mutex<Vec<PruvEvQueue>>, cv: ::std::sync::Condvar }
enum PruvEvResource { Free, Socket(::std::net::TcpStream, Option<::std::thread::JoinHandle<()>>), Listener(::std::net::SocketAddr, Option<::std::thread::JoinHandle<()>>, ::std::net::TcpListener), Process(pruv_Process), Timer(::std::time::Instant) }
#[allow(non_camel_case_types)]
struct pruv_Loop { shared: ::std::sync::Arc<PruvEvShared>, slots: Vec<PruvEvResource>, public: Vec<PruvEvPublic>, error: i64, first_err: i64, last: i64, event_slot: i64, event_kind: i64, cursor: usize, buf: Vec<u8>, exit: i64, writers: Vec<Option<PruvEvWriter>> }
struct PruvEvJob { data: Vec<u8>, off: usize, busy: bool, err: bool, stop: bool }
type PruvEvJobRef = ::std::sync::Arc<(::std::sync::Mutex<PruvEvJob>, ::std::sync::Condvar)>;
struct PruvEvWriter { job: PruvEvJobRef, h: Option<::std::thread::JoinHandle<()>> }
pub static PRUV_ASYNC_THREADS: ::std::sync::atomic::AtomicUsize = ::std::sync::atomic::AtomicUsize::new(0);
struct PruvEvThread;
impl PruvEvThread { fn new() -> Self { PRUV_ASYNC_THREADS.fetch_add(1, ::std::sync::atomic::Ordering::SeqCst); Self } }
impl Drop for PruvEvThread { fn drop(&mut self) { PRUV_ASYNC_THREADS.fetch_sub(1, ::std::sync::atomic::Ordering::SeqCst); } }
fn pruv_ev_remember(l: &mut pruv_Loop, e: i64) { if l.first_err == 0 { l.first_err = e; } }
fn pruv_ev_fail(l: &mut pruv_Loop, e: i64) { if l.error == 0 { l.error=e; } pruv_ev_remember(l,e); l.last=-1; l.event_slot=-1; l.event_kind=0; l.buf.clear(); }
fn pruv_ev_mark(l: &mut pruv_Loop, i: usize, e: i64) { pruv_ev_remember(l,e); l.public[i].failed=true; l.public[i].open=false; l.public[i].eof=false; l.shared.state.lock().unwrap()[i].reported=true; }
fn pruv_ev_new(cap: i64) -> pruv_Loop {
    let n = cap.clamp(1,4096) as usize;
    let mut l=pruv_Loop { shared: ::std::sync::Arc::new(PruvEvShared { state: ::std::sync::Mutex::new((0..n).map(|_| PruvEvQueue::default()).collect()), cv: ::std::sync::Condvar::new() }), slots: (0..n).map(|_| PruvEvResource::Free).collect(), public: vec![PruvEvPublic::default();n], error:0, first_err:0, last:-1, event_slot:-1, event_kind:0, cursor:n-1, buf:Vec::new(), exit:0, writers:(0..n).map(|_| None).collect() };
    if cap<1 || cap>4096 { pruv_ev_fail(&mut l,1); } l
}
fn pruv_ev_count(l: &pruv_Loop) -> i64 { l.public.iter().filter(|p|p.kind!=0).count() as i64 }
fn pruv_ev_valid(l: &pruv_Loop, slot: i64, kind: i64) -> bool { l.error==0 && slot>=0 && (slot as usize)<l.slots.len() && (kind==0 || l.public[slot as usize].kind==kind) }
fn pruv_ev_free(l: &pruv_Loop) -> Option<usize> { l.public.iter().position(|p|p.kind==0) }
fn pruv_ev_spawn<F: FnOnce()+Send+'static>(f: F) -> Option<::std::thread::JoinHandle<()>> {
    //@hook spawn return None;
    let guard=PruvEvThread::new();
    ::std::thread::Builder::new().spawn(move || { let _guard=guard; f(); }).ok()
}
const PRUV_EV_READ_CAP: usize = 1048576;
fn pruv_ev_reader(mut s: ::std::net::TcpStream, sh: ::std::sync::Arc<PruvEvShared>, i: usize) {
    let result=::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
        //@hook read { let mut q=sh.state.lock().unwrap(); q[i].err=3; sh.cv.notify_all(); return; }
        let mut b = vec![0u8; 65536];
        loop {
            let mut q=sh.state.lock().unwrap();
            while q[i].bytes[0].len()>=PRUV_EV_READ_CAP && !q[i].closing { q=sh.cv.wait(q).unwrap(); }
            if q[i].closing { return; }
            let limit = b.len().min(PRUV_EV_READ_CAP - q[i].bytes[0].len());
            drop(q);
            let r = ::std::io::Read::read(&mut s, &mut b[..limit]);
            let mut q=sh.state.lock().unwrap(); if q[i].closing { return; }
            match r { Ok(0)=>q[i].ended[0]=true, Ok(n)=>q[i].bytes[0].extend(&b[..n]), Err(e) if e.kind()==::std::io::ErrorKind::Interrupted=>continue, Err(_)=>q[i].err=3 }
            sh.cv.notify_all(); if q[i].ended[0] || q[i].err!=0 { return; }
        }
    }));
    if result.is_err() { sh.state.lock().unwrap()[i].err=3; sh.cv.notify_all(); }
}
// G10-ASYNC-DEADLINE: the stdin writer of a process slot; it writes 4096-byte pieces and checks stop between them
fn pruv_ev_writer(mut s: ::std::process::ChildStdin, job: PruvEvJobRef) {
    let (m,cv)=&*job;
    let result=::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
        //@hook write_for { let mut j=m.lock().unwrap(); j.err=true; j.busy=false; cv.notify_all(); return; }
        //@hook writer_panic panic!("pruv test hook: writer_panic");
        loop {
            let mut j=m.lock().unwrap_or_else(|e| e.into_inner());
            while !j.busy && !j.stop { j=cv.wait(j).unwrap_or_else(|e| e.into_inner()); }
            if j.stop { j.data=Vec::new(); return; }
            let a=j.off; let b=(a+4096).min(j.data.len()); let piece=j.data[a..b].to_vec(); drop(j);
            let r=::std::io::Write::write_all(&mut s,&piece);
            let mut j=m.lock().unwrap_or_else(|e| e.into_inner());
            match r { Ok(())=>{ j.off=b; if j.off>=j.data.len() { j.busy=false; j.data=Vec::new(); j.off=0; } cv.notify_all(); }, Err(_)=>{ j.err=true; j.busy=false; j.data=Vec::new(); cv.notify_all(); return; } }
        }
    }));
    if let Err(p)=result { { let mut j=m.lock().unwrap_or_else(|e| e.into_inner()); j.err=true; j.busy=false; j.data=Vec::new(); cv.notify_all(); } ::std::panic::resume_unwind(p); }
}
fn pruv_ev_writer_start(l: &mut pruv_Loop, i: usize) -> bool {
    if l.writers[i].is_some() { return true; }
    let s=match &mut l.slots[i] { PruvEvResource::Process(p)=>p.stdin.take(), _=>None };
    let s=match s { Some(s)=>s, None=>return false };
    let job: PruvEvJobRef=::std::sync::Arc::new((::std::sync::Mutex::new(PruvEvJob { data: Vec::new(), off: 0, busy: false, err: false, stop: false }), ::std::sync::Condvar::new()));
    let j2=job.clone();
    match pruv_ev_spawn(move || pruv_ev_writer(s,j2)) { Some(h)=>{ l.writers[i]=Some(PruvEvWriter { job, h: Some(h) }); true }, None=>false }
}
fn pruv_ev_writer_stop(l: &mut pruv_Loop, i: usize) { if let Some(w)=&l.writers[i] { let (m,cv)=&*w.job; m.lock().unwrap_or_else(|e| e.into_inner()).stop=true; cv.notify_all(); } }
// half-close: the slot keeps running and is not failed; stdin is released (by the writer after its current piece)
fn pruv_ev_half(l: &mut pruv_Loop, i: usize) { pruv_ev_writer_stop(l,i); if let PruvEvResource::Process(p)=&mut l.slots[i] { p.stdin=None; p.stdin_open=false; } l.public[i].open=false; }
fn pruv_ev_socket(mut l: pruv_Loop, mut s: pruv_Socket) -> pruv_Loop {
    l.last=-1;
    let i=match pruv_ev_free(&l) { Some(i) if l.error==0 && s.error==0=>i, _=>{ pruv_ev_fail(&mut l,1); return l; } };
    let stream=match s.stream.take() { Some(s)=>s, None=>{pruv_ev_fail(&mut l,2); return l;} };
    let reader=match stream.try_clone() { Ok(r) if stream.set_read_timeout(None).is_ok()=>r, _=>{pruv_ev_fail(&mut l,2);return l;} };
    l.shared.state.lock().unwrap()[i]=PruvEvQueue::default();
    let sh=l.shared.clone();
    let h=match pruv_ev_spawn(move || pruv_ev_reader(reader,sh,i)) {Some(h)=>h,None=>{pruv_ev_fail(&mut l,2);return l;}};
    l.slots[i]=PruvEvResource::Socket(stream,Some(h)); l.public[i]=PruvEvPublic {kind:1,open:s.write_open,..Default::default()}; l.last=i as i64; l
}
fn pruv_ev_listener(mut l: pruv_Loop, mut x: pruv_Listener) -> pruv_Loop {
    l.last=-1;
    let i=match pruv_ev_free(&l) {Some(i) if l.error==0 && x.error==0=>i,_=>{pruv_ev_fail(&mut l,1);return l;}};
    let lis=match x.listener.take() {Some(x)=>x,None=>{pruv_ev_fail(&mut l,2);return l;}};
    let local=match lis.local_addr() {Ok(a)=>a,Err(_)=>{pruv_ev_fail(&mut l,2);return l;}};
    let own=lis; let lis=match own.try_clone() {Ok(a)=>a,Err(_)=>{pruv_ev_fail(&mut l,2);return l;}};
    l.shared.state.lock().unwrap()[i]=PruvEvQueue::default(); let sh=l.shared.clone();
    let h=pruv_ev_spawn(move || {
        let result=::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
            //@hook accept { sh.state.lock().unwrap()[i].err=6; sh.cv.notify_all(); return; }
            loop { let mut q=sh.state.lock().unwrap(); while q[i].pending.len()>=64 && !q[i].closing {q=sh.cv.wait(q).unwrap();} if q[i].closing{return;} q[i].accepting=true; drop(q);
                let r=lis.accept(); let mut q=sh.state.lock().unwrap(); q[i].accepting=false; if q[i].closing{return;}
                match r {Ok((s,_))=>{let _=s.set_nodelay(true);q[i].pending.push_back(s);},Err(e) if e.kind()==::std::io::ErrorKind::Interrupted=>continue,Err(_)=>q[i].err=6} sh.cv.notify_all(); if q[i].err!=0{return;}
            }
        })); if result.is_err(){sh.state.lock().unwrap()[i].err=6;sh.cv.notify_all();}
    });
    match h {Some(h)=>{l.slots[i]=PruvEvResource::Listener(local,Some(h),own);l.public[i].kind=2;l.last=i as i64;},None=>pruv_ev_fail(&mut l,2)} l
}
fn pruv_ev_timer(mut l: pruv_Loop, ms: i64) -> pruv_Loop { l.last=-1; if let Some(i)=pruv_ev_free(&l) {if l.error==0 && (0..=86400000).contains(&ms) {l.slots[i]=PruvEvResource::Timer(::std::time::Instant::now()+::std::time::Duration::from_millis(ms as u64));l.public[i].kind=4;l.last=i as i64;return l;}} pruv_ev_fail(&mut l,1);l }
fn pruv_ev_process(mut l: pruv_Loop, mut p: pruv_Process) -> pruv_Loop {
    l.last=-1; let i=match pruv_ev_free(&l){Some(i) if l.error==0 && p.error==0=>i,_=>{let _=pruv_proc_finish(p,true);pruv_ev_fail(&mut l,1);return l;}};
    let mut forced=false;
    //@hook spawn forced=true;
    if forced {let _=pruv_proc_finish(p,true);pruv_ev_fail(&mut l,2);return l;}
    l.shared.state.lock().unwrap()[i]=PruvEvQueue::default();
    for (k,q) in [&mut p.outq,&mut p.errq].into_iter().enumerate() { if let Some(q)=q { q.rx.attach(l.shared.clone(),i,k); if q.off<q.rest.len(){let mut state=l.shared.state.lock().unwrap(); let mut front: ::std::collections::VecDeque<u8>=q.rest[q.off..].iter().copied().collect(); front.append(&mut state[i].bytes[k]);state[i].bytes[k]=front;q.rest.clear();q.off=0;} } }
    l.public[i]=PruvEvPublic{kind:3,open:p.stdin_open,..Default::default()};l.slots[i]=PruvEvResource::Process(p);l.last=i as i64;l
}
fn pruv_ev_wait(mut l: pruv_Loop, ms: i64) -> pruv_Loop {
    l.last=-1; if l.error!=0 || !(-1..=86400000).contains(&ms){pruv_ev_fail(&mut l,1);return l;}
    l.event_slot=-1;l.event_kind=0; let deadline=if ms<0 {None}else{Some(::std::time::Instant::now()+::std::time::Duration::from_millis(ms as u64))};
    loop {
        let sh=l.shared.clone(); let mut q=sh.state.lock().unwrap(); let now=::std::time::Instant::now(); let mut next=deadline; let mut event=None;
        for step in 1..=l.slots.len() {let i=(l.cursor+step)%l.slots.len();if l.public[i].failed{continue;}
            let kind=match &l.slots[i] {
                PruvEvResource::Free=>0,
                PruvEvResource::Timer(t)=>{next=Some(next.map_or(*t,|x|x.min(*t)));if *t<=now{4}else{0}},
                PruvEvResource::Listener(..)=>if !q[i].pending.is_empty(){2}else if q[i].err!=0 && !q[i].reported {5}else{0},
                _=>if q[i].bytes.iter().any(|b|!b.is_empty()){1}else if q[i].err!=0 && !q[i].reported {5}else if q[i].ended[0] && (l.public[i].kind==1 || q[i].ended[1]) && !q[i].reported {3}else{0},
            }; if kind!=0 {event=Some((i,kind));break;}
        }
        if let Some((i,k))=event {l.cursor=i;l.event_slot=i as i64;l.event_kind=k;if k==3 {q[i].reported=true;} let err=q[i].err;drop(q);if k==5{pruv_ev_mark(&mut l,i,err);}if k==4{l.slots[i]=PruvEvResource::Free;l.public[i]=PruvEvPublic::default();}return l;}
        if deadline.map_or(false,|d|now>=d){return l;}
        q=if let Some(d)=next {sh.cv.wait_timeout(q,d.saturating_duration_since(now)).unwrap().0}else{sh.cv.wait(q).unwrap()};drop(q);
    }
}
fn pruv_ev_read(mut l: pruv_Loop, slot: i64, stream: i64, max: i64, proc: bool) -> pruv_Loop {
    l.buf.clear();
    if !(1..=(1i64<<40)).contains(&max) { pruv_ev_fail(&mut l, 1); return l; }
    pruv_ev_read_queue(l, slot, stream, max.min(1048576) as usize, proc, None).0
}
fn pruv_ev_read_into(l: pruv_Loop, slot: i64, stream: i64, dst: &mut [u8], n: &mut i64, proc: bool) -> pruv_Loop {
    let (l, count) = pruv_ev_read_queue(l, slot, stream, dst.len(), proc, Some(dst));
    *n = count as i64;
    l
}
fn pruv_ev_read_queue(mut l: pruv_Loop, slot: i64, stream: i64, max: usize, proc: bool, dst: Option<&mut [u8]>) -> (pruv_Loop, usize) {
    l.last = -1;
    if !pruv_ev_valid(&l, slot, if proc { 3 } else { 1 }) || !(1..=2).contains(&stream) {
        pruv_ev_fail(&mut l, 1);
        return (l, 0);
    }
    let i = slot as usize;
    let k = (stream - 1) as usize;
    if l.public[i].failed { return (l, 0); }
    let sh = l.shared.clone();
    let mut q = sh.state.lock().unwrap();
    let bytes = &mut q[i].bytes[k];
    let n = max.min(bytes.len());
    if n > 0 {
        if let Some(dst) = dst {
            let (front, back) = bytes.as_slices();
            let first = n.min(front.len());
            dst[..first].copy_from_slice(&front[..first]);
            dst[first..n].copy_from_slice(&back[..n - first]);
            drop(bytes.drain(..n));
        } else {
            let (front, back) = bytes.as_slices();
            let first = n.min(front.len());
            l.buf.extend_from_slice(&front[..first]);
            l.buf.extend_from_slice(&back[..n - first]);
            drop(bytes.drain(..n));
        }
        sh.cv.notify_all();
    } else if bytes.is_empty() {
        if q[i].err != 0 {
            let e = q[i].err;
            drop(q);
            pruv_ev_mark(&mut l, i, e);
            return (l, 0);
        }
        if q[i].ended[k] {
            l.public[i].ends[k] = true;
            l.public[i].eof = l.public[i].ends[0] && (!proc || l.public[i].ends[1]);
            if l.public[i].eof { q[i].reported = true; }
        }
    }
    (l, n)
}
fn pruv_ev_write(mut l: pruv_Loop, slot: i64, data: &[u8], action: i64) -> pruv_Loop {
    l.last=-1;let proc=action>=2; if !pruv_ev_valid(&l,slot,if proc{3}else{1}) || !l.public[slot as usize].open {pruv_ev_fail(&mut l,1);return l;}let i=slot as usize;let mut code=0;
    //@hook write if !proc {code=4;}
    //@hook write_timeout if !proc {code=5;}
    //@hook proc_io if proc {code=7;}
    if code==0 && proc && l.writers[i].is_some() {
        let job=l.writers[i].as_ref().unwrap().job.clone(); let (m,cv)=&*job; let mut j=m.lock().unwrap_or_else(|e| e.into_inner());
        if action==3 { j.stop=true; cv.notify_all(); } else if j.err { code=7; } else if !data.is_empty() { j.data=data.to_vec(); j.off=0; j.busy=true; cv.notify_all(); while j.busy && !j.err { j=cv.wait(j).unwrap_or_else(|e| e.into_inner()); } if j.err { code=7; } }
    } else if code==0 { match &mut l.slots[i] {
        PruvEvResource::Socket(s,_)=>{let r=if action==1{s.shutdown(::std::net::Shutdown::Write)}else{::std::io::Write::write_all(s,data)};if let Err(e)=r{code=if action==0 && pruv_net_timed_out(&e){5}else{4};}},
        PruvEvResource::Process(p)=>{if action==3 {p.stdin=None;}else if p.stdin.as_mut().map_or(true,|s|::std::io::Write::write_all(s,data).is_err()){code=7;}},_=>{}
    }} if code!=0{let eof=l.public[i].eof;pruv_ev_mark(&mut l,i,code);l.public[i].eof=eof;}else if action==1 || action==3 {l.public[i].open=false;}l
}
fn pruv_ev_write_for(mut l: pruv_Loop, slot: i64, data: &[u8], ms: i64) -> pruv_Loop {
    l.last=-1; if !pruv_ev_valid(&l,slot,3) || !l.public[slot as usize].open || !(0..=86400000).contains(&ms) {pruv_ev_fail(&mut l,1);return l;}
    let i=slot as usize; if data.is_empty() {return l;}
    let mut ok=pruv_ev_writer_start(&mut l,i);
    if ok {
        let job=l.writers[i].as_ref().unwrap().job.clone(); let (m,cv)=&*job; let mut j=m.lock().unwrap_or_else(|e| e.into_inner());
        if j.err { ok=false; } else {
            j.data=data.to_vec(); j.off=0; j.busy=true; cv.notify_all();
            let deadline=::std::time::Instant::now()+::std::time::Duration::from_millis(ms as u64);
            loop { if j.err {ok=false;break;} if !j.busy {break;} let now=::std::time::Instant::now(); if now>=deadline {ok=false;j.stop=true;j.data=Vec::new();cv.notify_all();break;} j=cv.wait_timeout(j,deadline-now).unwrap_or_else(|e| e.into_inner()).0; }
        }
    }
    if !ok { pruv_ev_half(&mut l,i); } l
}
fn pruv_ev_wait_for(mut l: pruv_Loop, slot: i64, ms: i64) -> pruv_Loop {
    l.last=-1; if !pruv_ev_valid(&l,slot,3) || !(0..=86400000).contains(&ms) {pruv_ev_fail(&mut l,1);return l;}
    let i=slot as usize; pruv_ev_half(&mut l,i);
    if l.public[i].failed { pruv_ev_end(&mut l,i,true); return l; }
    let deadline=::std::time::Instant::now()+::std::time::Duration::from_millis(ms as u64); let mut step: u64=1;
    loop {
        let exited=match &mut l.slots[i] { PruvEvResource::Process(p)=>p.child.as_mut().map_or(true,|c| !matches!(c.try_wait(),Ok(None))), _=>true };
        let sh=l.shared.clone(); let q=sh.state.lock().unwrap();
        if exited && (q[i].err!=0 || (q[i].ended[0] && q[i].ended[1])) { drop(q); pruv_ev_end(&mut l,i,true); return l; }
        let now=::std::time::Instant::now(); if now>=deadline { return l; }
        let q=sh.cv.wait_timeout(q,::std::time::Duration::from_millis(step).min(deadline-now)).unwrap().0; drop(q); step=(step*2).min(10);
    }
}
fn pruv_ev_accept(mut l: pruv_Loop, slot: i64) -> pruv_Loop {l.last=-1;if !pruv_ev_valid(&l,slot,2) || pruv_ev_free(&l).is_none(){pruv_ev_fail(&mut l,1);return l;}let i=slot as usize;if l.public[i].failed{return l;}let sh=l.shared.clone();let mut q=sh.state.lock().unwrap();let s=q[i].pending.pop_front();let err=q[i].err;sh.cv.notify_all();drop(q);if let Some(s)=s {return pruv_ev_socket(l,pruv_Socket{stream:Some(s),error:0,write_open:true,eof:false,buf:Vec::new()});}if err!=0{pruv_ev_mark(&mut l,i,err);}l}
fn pruv_ev_end(l: &mut pruv_Loop, i: usize, wait: bool) {
    let accepting={let mut q=l.shared.state.lock().unwrap();q[i].closing=true;l.shared.cv.notify_all();q[i].accepting};
    let mut code=0;
    match ::std::mem::replace(&mut l.slots[i],PruvEvResource::Free) {
        PruvEvResource::Socket(s,h)=>{let _=s.shutdown(::std::net::Shutdown::Both);if h.map_or(false,|h|h.join().is_err()){code=3;}},
        PruvEvResource::Listener(mut a,h,own)=>{if a.ip().is_unspecified(){a.set_ip(if a.is_ipv4(){"127.0.0.1".parse().unwrap()}else{"::1".parse().unwrap()});}if accepting && ::std::net::TcpStream::connect_timeout(&a,::std::time::Duration::from_secs(1)).is_err(){code=2;}else if h.map_or(false,|h|h.join().is_err()){code=6;}drop(own);},
        PruvEvResource::Process(p)=>{pruv_ev_writer_stop(l,i);let w=l.writers[i].take();let kill=!wait || l.public[i].failed;let r=pruv_proc_finish(p,kill);match r {Ok(n)=>if wait{l.exit=n;},Err(_)=>{code=8;if wait{l.exit=0;}}}
            if let Some(mut w)=w {if w.h.take().map_or(false,|h|h.join().is_err()){code=8;}}
            //@hook reap code=8;
        },_=>{}
    }
    l.public[i]=PruvEvPublic::default();l.shared.state.lock().unwrap()[i]=PruvEvQueue::default();if code!=0{pruv_ev_remember(l,code);if code==2 {pruv_ev_fail(l,2);}}
}
fn pruv_ev_remove(mut l: pruv_Loop, slot: i64, wait: bool) -> pruv_Loop {l.last=-1;if !pruv_ev_valid(&l,slot,if wait{3}else{0}) || l.public[slot as usize].kind==0 {pruv_ev_fail(&mut l,1);return l;}pruv_ev_end(&mut l,slot as usize,wait);l}
fn pruv_ev_close(mut l: pruv_Loop) -> Result<i64,i64> {let n=pruv_ev_count(&l);for i in 0..l.slots.len(){if l.public[i].kind!=0{pruv_ev_end(&mut l,i,false);}}if l.first_err!=0 {Err(l.first_err)}else{Ok(n)}}
impl Drop for pruv_Loop {fn drop(&mut self){for i in 0..self.slots.len(){if self.public[i].kind!=0{pruv_ev_end(self,i,false);}}}}
fn pruv_ev_post(op:&str, s:&PruvEvSnapshot, r:&pruv_Loop, slot:i64)->bool {
    if !pruv_ev_wf(r) || r.public.len()!=s.public.len(){return false;}
    if r.error!=0{return r.last == -1 && r.event_slot == -1 && r.event_kind == 0 && r.buf.is_empty();}
    if op!="ev_wait" && (r.event_slot!=s.event_slot || r.event_kind!=s.event_kind){return false;}
    if op!="ev_proc_wait" && op!="ev_proc_wait_for" && r.exit!=s.exit{return false;}
    if op!="ev_read" && op!="ev_proc_read" && r.buf!=s.buf{return false;}
    if !op.starts_with("ev_add_") && op!="ev_accept" && r.last != -1{return false;}
    let changed=if op.starts_with("ev_add_") || op=="ev_accept" {r.last}else if op=="ev_wait"{if r.event_kind==4 || r.event_kind==5{r.event_slot}else{-1}}else{slot};
    for (i,(a,b)) in s.public.iter().zip(&r.public).enumerate(){if i as i64!=changed && (a.kind!=b.kind || a.open!=b.open || a.eof!=b.eof){return false;}}
    true
}

// The route mutex serializes collector attachment only; attached bytes and readiness use the loop mutex.
// Before attachment, standalone proc operations retain their unbounded collector semantics.
type PruvEvRoute = ::std::sync::Arc<::std::sync::Mutex<(::std::sync::Arc<PruvEvShared>,usize,usize,bool)>>;
struct PruvEvRx { route: PruvEvRoute }
struct PruvEvTx { route: PruvEvRoute, discard: ::std::sync::Arc<::std::sync::atomic::AtomicBool> }
impl PruvEvTx {
    fn send(&self, msg: Result<Option<&[u8]>, ()>) {
        loop {
            let route = self.route.lock().unwrap();
            let (sh, i, k, attached) = &*route;
            let sh = sh.clone();
            let mut q = sh.state.lock().unwrap();
            match msg {
                Ok(Some(bytes)) => {
                    if self.discard.load(::std::sync::atomic::Ordering::SeqCst) || q[*i].closing { return; }
                    // Keep the in-flight batch in the reader buffer until there is room for it.
                    if *attached && q[*i].bytes[*k].len() + bytes.len() > PRUV_EV_READ_CAP {
                        drop(route);
                        drop(sh.cv.wait_timeout(q, ::std::time::Duration::from_millis(10)).unwrap());
                        continue;
                    }
                    q[*i].bytes[*k].extend(bytes);
                }
                Ok(None) => q[*i].ended[*k] = true,
                Err(()) => q[*i].err = 7,
            }
            sh.cv.notify_all();
            return;
        }
    }
}
impl Drop for PruvEvTx {
    fn drop(&mut self) {
        let route = self.route.lock().unwrap();
        let (sh, i, k, _) = &*route;
        let mut q = sh.state.lock().unwrap();
        if !q[*i].ended[*k] && q[*i].err == 0 {
            q[*i].err = 7;
            sh.cv.notify_all();
        }
    }
}
impl PruvEvRx {
    fn attach(&mut self, target: ::std::sync::Arc<PruvEvShared>, slot: usize, stream: usize) {
        let mut route=self.route.lock().unwrap();let (sh,i,k,_)=&*route;let mut old=sh.state.lock().unwrap();let mut q=target.state.lock().unwrap();
        q[slot].bytes[stream]=::std::mem::take(&mut old[*i].bytes[*k]);q[slot].ended[stream]=old[*i].ended[*k];if old[*i].err!=0{q[slot].err=7;}drop(q);drop(old);
        *route=(target.clone(),slot,stream,true);target.cv.notify_all();
    }
    fn recv(&self) -> Result<PruvProcMsg,()> {
        loop {let route=self.route.lock().unwrap();let (shared,i,k,attached)=&*route;let sh=shared.clone();let mut q=sh.state.lock().unwrap();
            if !q[*i].bytes[*k].is_empty(){let n=q[*i].bytes[*k].len().min(8192);let b=q[*i].bytes[*k].drain(..n).collect();sh.cv.notify_all();return Ok(PruvProcMsg::Chunk(b));}
            if q[*i].err!=0{return Ok(PruvProcMsg::Err);}if q[*i].ended[*k]{return Ok(PruvProcMsg::Eof);}
            let sh=sh.clone();drop(route);q=sh.cv.wait(q).unwrap();drop(q);
        }
    }
}
#[derive(Clone)]
struct PruvEvSnapshot { public: Vec<PruvEvPublic>, error:i64, last:i64, event_slot:i64, event_kind:i64, buf:Vec<u8>, exit:i64 }
fn pruv_ev_snapshot(l:&pruv_Loop)->PruvEvSnapshot {PruvEvSnapshot{public:l.public.clone(),error:l.error,last:l.last,event_slot:l.event_slot,event_kind:l.event_kind,buf:l.buf.clone(),exit:l.exit}}
fn pruv_ev_wf(l:&pruv_Loop)->bool {(0..=2).contains(&l.error) && l.slots.len()>=1 && l.slots.len()<=4096 && l.last>=-1 && l.last<(l.slots.len() as i64) && l.event_slot>=-1 && l.event_slot<(l.slots.len() as i64) && (0..=5).contains(&l.event_kind) && (l.event_slot == -1)==(l.event_kind==0) && l.public.iter().all(|p| (0..=4).contains(&p.kind) && (!p.open && !p.eof || p.kind==1 || p.kind==3))}
fn pruv_cev_slot(l:&pruv_Loop,i:i128)->usize {if i<0 || i>=l.public.len() as i128 {pruv_ud("loop slot out of range");}i as usize}
"""
ASYNC_PROC_PRELUDE = r"""#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
struct pruv_ProcessArgs { args: ::std::vec::Vec<::std::string::String> }
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
enum PruvProcMsg { Chunk(::std::vec::Vec<u8>), Eof, Err }
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
struct PruvProcStream {
    rx: PruvEvRx,
    rest: ::std::vec::Vec<u8>,
    off: usize,
    done: bool,
    failed: bool,
    join: ::std::option::Option<::std::thread::JoinHandle<()>>,
    discard: ::std::sync::Arc<::std::sync::atomic::AtomicBool>,
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
struct pruv_Process {
    child: ::std::option::Option<::std::process::Child>,
    stdin: ::std::option::Option<::std::process::ChildStdin>,
    outq: ::std::option::Option<PruvProcStream>,
    errq: ::std::option::Option<PruvProcStream>,
    error: i64,
    stdin_open: bool,
    stdout_eof: bool,
    stderr_eof: bool,
    stdout_buf: ::std::vec::Vec<u8>,
    stderr_buf: ::std::vec::Vec<u8>,
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
impl ::std::ops::Drop for pruv_Process {
    fn drop(&mut self) {
        // defensive only (must-consume is the contract): an unconsumed handle still kills and reaps its direct child
        self.stdin = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
#[cfg(unix)]
fn pruv_proc_sigpipe() {
    extern "C" { fn signal(signum: i32, handler: usize) -> usize; }
    static PRUV_PROC_ONCE: ::std::sync::Once = ::std::sync::Once::new();
    // a write to a closed pipe is the error 4, never a SIGPIPE death of the Pruv process (SIGPIPE = 13, SIG_IGN = 1)
    PRUV_PROC_ONCE.call_once(|| { unsafe { signal(13, 1); } });
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
#[cfg(not(unix))]
fn pruv_proc_sigpipe() {}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
#[cfg(windows)]
fn pruv_proc_batch(program: &str) -> bool {
    // Windows drops trailing dots and spaces from a file name, so x.bat. and x.bat followed by a space name it too
    let l = program.trim_end_matches(|c: char| c == '.' || c == ' ').to_ascii_lowercase();
    l.ends_with(".bat") || l.ends_with(".cmd")
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
#[cfg(not(windows))]
fn pruv_proc_batch(program: &str) -> bool { false }
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
#[cfg(unix)]
fn pruv_proc_status(st: ::std::process::ExitStatus) -> i64 {
    match st.code() {
        Some(c) => c as i64,
        None => match ::std::os::unix::process::ExitStatusExt::signal(&st) { Some(s) => -(s as i64), None => -1 },
    }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
#[cfg(not(unix))]
fn pruv_proc_status(st: ::std::process::ExitStatus) -> i64 {
    match st.code() { Some(c) => (c as u32) as i64, None => -1 }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_failed(code: i64) -> pruv_Process {
    pruv_Process { child: None, stdin: None, outq: None, errq: None, error: code, stdin_open: false, stdout_eof: false, stderr_eof: false, stdout_buf: ::std::vec::Vec::new(), stderr_buf: ::std::vec::Vec::new() }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_fail(p: &mut pruv_Process, code: i64) {
    // the first published error is kept; an update failure closes stdin and empties the public state
    if p.error == 0 { p.error = code; }
    p.stdin = None;
    p.stdin_open = false;
    p.stdout_buf = ::std::vec::Vec::new();
    p.stderr_buf = ::std::vec::Vec::new();
    p.stdout_eof = false;
    p.stderr_eof = false;
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_reader<R: ::std::io::Read>(mut r: R, tx: PruvEvTx, stream: i64) {
    let mut buf: ::std::vec::Vec<u8> = vec![0u8; 8192];
    //@hook read1 if stream == 1 { tx.send(Err(())); while let Ok(n) = ::std::io::Read::read(&mut r, &mut buf) { if n == 0 { break; } } return; }
    //@hook read2 if stream == 2 { tx.send(Err(())); while let Ok(n) = ::std::io::Read::read(&mut r, &mut buf) { if n == 0 { break; } } return; }
    //@async_hook proc_io { tx.send(Err(())); return; }
    loop {
        match ::std::io::Read::read(&mut r, &mut buf) {
            Ok(0) => { tx.send(Ok(None)); return; }
            Ok(n) => tx.send(Ok(Some(&buf[..n]))),
            Err(e) => {
                if e.kind() == ::std::io::ErrorKind::Interrupted { continue; }
                tx.send(Err(()));
                return;
            }
        }
    }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_collector<R: ::std::io::Read + Send + 'static>(r: R, stream: i64) -> ::std::option::Option<PruvProcStream> {
    let discard = ::std::sync::Arc::new(::std::sync::atomic::AtomicBool::new(false));
    let shared=pruv_ev_new(1).shared.clone();
    let route=::std::sync::Arc::new(::std::sync::Mutex::new((shared,0,0,false)));
    let rx=PruvEvRx {route:route.clone()};
    let tx=PruvEvTx {route,discard:discard.clone()};
    let join = match ::std::thread::Builder::new().spawn(move || { let _guard=PruvEvThread::new(); pruv_proc_reader(r, tx, stream) }) {
        Ok(h) => h,
        Err(_) => return None,
    };
    Some(PruvProcStream { rx, rest: ::std::vec::Vec::new(), off: 0, done: false, failed: false, join: Some(join), discard })
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_spawn(program: &str, args: &pruv_ProcessArgs, cwd: &str) -> pruv_Process {
    pruv_proc_spawn_in(program, args, cwd, &[], &[])
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_spawn_env(program: &str, args: &pruv_ProcessArgs, cwd: &str, names: &pruv_ProcessArgs, values: &pruv_ProcessArgs) -> pruv_Process {
    pruv_proc_spawn_in(program, args, cwd, &names.args, &values.args)
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_spawn_in(program: &str, args: &pruv_ProcessArgs, cwd: &str, names: &[::std::string::String], values: &[::std::string::String]) -> pruv_Process {
    pruv_proc_sigpipe();
    if program.is_empty() || program.contains('\0') || cwd.contains('\0') || args.args.iter().any(|a| a.contains('\0')) {
        return pruv_proc_failed(1);
    }
    if names.len() != values.len() || names.iter().any(|k| k.is_empty() || k.contains('=') || k.contains('\0')) || values.iter().any(|v| v.contains('\0')) {
        return pruv_proc_failed(1);
    }
    if pruv_proc_batch(program) {
        return pruv_proc_failed(1);
    }
    let mut cmd = ::std::process::Command::new(program);
    cmd.args(args.args.iter());
    for (k, v) in names.iter().zip(values.iter()) {
        cmd.env(k, v);
    }
    if !cwd.is_empty() {
        cmd.current_dir(cwd);
    }
    cmd.stdin(::std::process::Stdio::piped());
    cmd.stdout(::std::process::Stdio::piped());
    cmd.stderr(::std::process::Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return pruv_proc_failed(2),
    };
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let mut outq: ::std::option::Option<PruvProcStream> = None;
    let mut errq: ::std::option::Option<PruvProcStream> = None;
    let mut bad: bool = stdin.is_none() || stdout.is_none() || stderr.is_none();
    //@hook setup bad = true;
    if !bad {
        outq = match stdout { Some(r) => pruv_proc_collector(r, 1), None => None };
        errq = match stderr { Some(r) => pruv_proc_collector(r, 2), None => None };
        bad = outq.is_none() || errq.is_none();
        //@hook setup_late bad = true;
    }
    if bad {
        // setup failed after the spawn: release stdin, kill and reap the direct child, stop and join the started readers
        drop(stdin);
        let _ = child.kill();
        let _ = child.wait();
        if let Some(q) = outq.take() { pruv_proc_discard(&q); let _ = pruv_proc_finish_stream(q); }
        if let Some(q) = errq.take() { pruv_proc_discard(&q); let _ = pruv_proc_finish_stream(q); }
        return pruv_proc_failed(2);
    }
    pruv_Process { child: Some(child), stdin, outq, errq, error: 0, stdin_open: true, stdout_eof: false, stderr_eof: false, stdout_buf: ::std::vec::Vec::new(), stderr_buf: ::std::vec::Vec::new() }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_take(q: &mut PruvProcStream, max: usize) -> ::std::option::Option<::std::vec::Vec<u8>> {
    // at most max bytes of the queue; blocks until bytes, EOF or an error; None = the stream failed
    loop {
        if q.off < q.rest.len() {
            let end: usize = q.off + ::std::cmp::min(q.rest.len() - q.off, max);
            let out: ::std::vec::Vec<u8> = q.rest[q.off..end].to_vec();
            q.off = end;
            if q.off == q.rest.len() {
                q.rest.clear();
                q.off = 0;
            }
            return Some(out);
        }
        if q.done {
            return Some(::std::vec::Vec::new());
        }
        if q.failed {
            return None;
        }
        match q.rx.recv() {
            Ok(PruvProcMsg::Chunk(c)) => { q.rest = c; q.off = 0; }
            Ok(PruvProcMsg::Eof) => { q.done = true; }
            Ok(PruvProcMsg::Err) | Err(_) => { q.failed = true; }
        }
    }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_collect(q: &mut PruvProcStream, out: &mut ::std::vec::Vec<u8>) -> bool {
    // everything not yet delivered, up to EOF; false = the stream failed
    loop {
        if q.off < q.rest.len() {
            out.extend_from_slice(&q.rest[q.off..]);
            q.rest.clear();
            q.off = 0;
        }
        if q.done {
            return true;
        }
        if q.failed {
            return false;
        }
        match q.rx.recv() {
            Ok(PruvProcMsg::Chunk(c)) => { q.rest = c; q.off = 0; }
            Ok(PruvProcMsg::Eof) => { q.done = true; }
            Ok(PruvProcMsg::Err) | Err(_) => { q.failed = true; }
        }
    }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_discard(q: &PruvProcStream) {
    q.discard.store(true, ::std::sync::atomic::Ordering::SeqCst);
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_finish_stream(mut q: PruvProcStream) -> bool {
    // discard the rest of a stream up to EOF, then join its reader thread; false = the stream failed
    let mut ok: bool = true;
    loop {
        if q.done { break; }
        if q.failed { ok = false; break; }
        match q.rx.recv() {
            Ok(PruvProcMsg::Chunk(_)) => {}
            Ok(PruvProcMsg::Eof) => { q.done = true; }
            Ok(PruvProcMsg::Err) | Err(_) => { q.failed = true; }
        }
    }
    if let Some(h) = q.join.take() {
        if h.join().is_err() { ok = false; }
    }
    ok
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_write(mut p: pruv_Process, data: &[u8]) -> pruv_Process {
    if p.error != 0 { return p; }
    let ok: bool = match p.stdin.as_mut() {
        Some(w) => ::std::io::Write::write_all(w, data).is_ok(),
        None => false,
    };
    if !ok { pruv_proc_fail(&mut p, 4); }
    p
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_close(mut p: pruv_Process) -> pruv_Process {
    if p.error != 0 { return p; }
    let mut bad: bool = p.stdin.is_none();
    //@hook close bad = true;
    p.stdin = None;
    p.stdin_open = false;
    if bad { pruv_proc_fail(&mut p, 4); }
    p
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_read(mut p: pruv_Process, stream: i64, max: i64) -> pruv_Process {
    if stream != 1 && stream != 2 { pruv_g6_repr("stream"); }
    if max <= 0 || max > (1i64 << 40) { pruv_g6_repr("max"); }
    if p.error != 0 { return p; }
    let lim: usize = ::std::cmp::min(max as u64, usize::MAX as u64) as usize;
    let got: ::std::option::Option<::std::vec::Vec<u8>> = {
        let q = if stream == 1 { p.outq.as_mut() } else { p.errq.as_mut() };
        match q { Some(q) => pruv_proc_take(q, lim), None => None }
    };
    match got {
        Some(chunk) => {
            let eof: bool = chunk.is_empty();
            if stream == 1 { p.stdout_buf = chunk; p.stdout_eof = eof; } else { p.stderr_buf = chunk; p.stderr_eof = eof; }
        }
        None => { pruv_proc_fail(&mut p, 3); }
    }
    p
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_read_all(mut p: pruv_Process) -> pruv_Process {
    if p.error != 0 { return p; }
    let mut ok: bool = true;
    let mut a: ::std::vec::Vec<u8> = ::std::vec::Vec::new();
    let mut b: ::std::vec::Vec<u8> = ::std::vec::Vec::new();
    // fixed publication order: stdout, then stderr (both are always drained to EOF)
    match p.outq.as_mut() { Some(q) => { if !pruv_proc_collect(q, &mut a) { ok = false; } } None => { ok = false; } }
    match p.errq.as_mut() { Some(q) => { if !pruv_proc_collect(q, &mut b) { ok = false; } } None => { ok = false; } }
    if ok {
        p.stdout_buf = a;
        p.stderr_buf = b;
        p.stdout_eof = true;
        p.stderr_eof = true;
    } else {
        pruv_proc_fail(&mut p, 3);
    }
    p
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_finish(mut p: pruv_Process, kill: bool) -> ::std::result::Result<i64, i64> {
    // wait / kill: release stdin, (kill,) reap the direct child, discard the rest of both outputs and join the readers
    p.stdin = None;
    p.stdin_open = false;
    let mut code: i64 = 0;
    let mut op_err: i64 = 0;
    // both collectors stop queueing before the wait, so output the child writes meanwhile is not held in memory
    if let Some(q) = p.outq.as_ref() { pruv_proc_discard(q); }
    if let Some(q) = p.errq.as_ref() { pruv_proc_discard(q); }
    if let Some(mut c) = p.child.take() {
        let mut kbad: bool = false;
        if kill {
            let exited: bool = match c.try_wait() { Ok(Some(_)) => true, _ => false };
            if !exited {
                if c.kill().is_err() {
                    // the child may have exited between the check and the request: that race is not a failure
                    kbad = match c.try_wait() { Ok(Some(_)) => false, _ => true };
                }
            }
            //@hook kill kbad = true;
        }
        let mut st = c.wait();
        //@hook wait st = Err(::std::io::Error::new(::std::io::ErrorKind::Other, "pruv test wait fault"));
        match st {
            Ok(s) => { code = pruv_proc_status(s); if kbad { op_err = 6; } }
            Err(_) => { op_err = if kbad { 6 } else { 5 }; }
        }
    }
    let mut cbad: bool = false;
    if let Some(q) = p.outq.take() { if !pruv_proc_finish_stream(q) { cbad = true; } }
    if let Some(q) = p.errq.take() { if !pruv_proc_finish_stream(q) { cbad = true; } }
    let err: i64 = if p.error != 0 { p.error } else if op_err != 0 { op_err } else if cbad { 3 } else { 0 };
    if err != 0 { Err(err) } else { Ok(code) }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_buf(p: &pruv_Process, stream: i64) -> &::std::vec::Vec<u8> {
    if stream == 1 { &p.stdout_buf } else if stream == 2 { &p.stderr_buf } else { pruv_g6_repr("stream") }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_wf(p: &pruv_Process) -> bool {
    p.error >= 0 && p.error <= 6 && (p.error == 0 || (!p.stdin_open && p.stdout_buf.is_empty() && p.stderr_buf.is_empty() && !p.stdout_eof && !p.stderr_eof))
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_args_new() -> pruv_ProcessArgs { pruv_ProcessArgs { args: ::std::vec::Vec::new() } }
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_proc_args_push(mut a: pruv_ProcessArgs, arg: &str) -> pruv_ProcessArgs { a.args.push(::std::string::String::from(arg)); a }
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_cproc_buf(p: &pruv_Process, stream: i128) -> &::std::vec::Vec<u8> {
    if stream == 1 { &p.stdout_buf } else if stream == 2 { &p.stderr_buf } else { pruv_ud("process stream outside 1..2") }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_cproc_eof(p: &pruv_Process, stream: i128) -> bool {
    if stream == 1 { p.stdout_eof } else if stream == 2 { p.stderr_eof } else { pruv_ud("process stream outside 1..2") }
}
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables, unused_mut, unused_assignments)]
fn pruv_cproc_byte(p: &pruv_Process, stream: i128, i: i128) -> i128 {
    let b = pruv_cproc_buf(p, stream);
    if i >= 0 && i < (b.len() as i128) { b[i as usize] as i128 } else { pruv_ud("process byte index out of range") }
}
"""

def async_filter(src, hook, prefix="//@hook"):
    out=[]
    for line in src.split("\n"):
        x=line.strip().split(" ",2)
        if line.strip().startswith(prefix+" "):
            if len(x)==3 and x[1]==hook: out.append(line[:len(line)-len(line.lstrip())]+x[2])
        else: out.append(line)
    return "\n".join(out)

def async_prelude(hook):
    return async_filter(ASYNC_PRELUDE,hook)

def async_proc_prelude(proc_hook, hook):
    return async_filter(async_filter(ASYNC_PROC_PRELUDE,proc_hook),hook,"//@async_hook")
