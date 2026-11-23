"""Pruv -> LLVM IR (text) + Rust bindings.  gen_llvm(prog, verified=True, trust_callers=False, budget=1 << 30)
returns (ir_text, rust_bindings_text).  Semantics reference: codegen.py (Rust back end);
format spec: docs/M1_LLVM_SPEC.md.

Modes (two independent axes, same meaning as codegen.py):
  verified=True   Output of a program the verifier proved: + - * are `add/sub/mul nsw`, unary - is
                  `sub nsw 0, x`, / and % are `udiv/urem` (verifier proved dividend >= 0, divisor > 0),
                  array accesses are `getelementptr inbounds` without checks.  No checks at all.
  verified=False  (--no-verify, "safe mode") nothing is assumed: + - * and unary - use
                  llvm.s*.with.overflow.i64 (fail -> pruv_panic_arith "overflow in +" ...), / and %
                  check `divisor > 0 && dividend >= 0` (fail -> "division outside domain: ..."),
                  index read/write checks 0 <= i < len (fail -> pruv_panic_oob), alloc(n, v) checks
                  0 <= n <= 2^40.  No `nsw` is ever emitted in safe mode.
  trust_callers=False (default)  the entry function checks every `requires` (i128) before the body;
                  trust_callers=True: entry calls the body without checks.

Generated names (user identifiers never start with `pruv_` nor contain `__`, so no collisions):
  @f__pruv_body    internal; the function body.  Inner calls: verified -> f__pruv_body (the caller proved
                  requires), safe -> f__pruv_entry.
  @f__pruv_entry   internal; evaluates requires (declaration order, first failure panics with
                  pruv_panic_requires(fn, show(expr))) and calls the body.
  @pruv_abi_f      external C-ABI wrapper used by the Rust bindings (bool -> i64, Option/Result args as
                  two i64, array results through a trailing `ptr out`, Option/Result results by value as { i64, i64 }).
  @pruv_cdiv/@pruv_crem  internal i128 Euclidean division for requires (only if some entry checks).
  @.str.N         private string constants.
  G9 A3 helpers   @pruv_drop__T / @pruv_clone__T / @pruv_box__T / @pruv_dropnode__T / @pruv_clonenode__T (extended
                  types), @pruv_csize__T / @pruv_ceq__T (runtime contracts); see the G9 A3 design note before class Words.
G4 A3 (explicit visibility mode, docs/G4_SPEC.md 4, 7; Program.explicit): the program is the mono plan, so every function and
type already carries its Rust stem (same stems, order and entry set as codegen.py).  Bodies and checked `__pruv_entry`
wrappers are `internal`; `pruv_abi_<entry>` (and `__typed`) exist only for host entries (root `pub`, nongeneric).  Verified:
only a host entry with a requires has a checked `__pruv_entry` (none under trust_callers) and every Pruv call goes to the
body; safe mode: every function with a requires has one and Pruv calls go through it, also under trust_callers (which only
makes the host-boundary `pruv_abi_*` call the body directly).  Representation checks in `pruv_abi_*` run in every mode.
Local IR names: %tN temporaries, %lN.name.{a,b} local slots (entry-block alloca), %p.name parameters,
%qN.* quantifier slots, labels `hint.N` (per-function counters).

Ownership of arrays is tracked at compile time (borrowed params; alloc/copy/array-returning call
results are owned).  Block-scoped locals are freed (pruv_free) in reverse declaration order when the
block ends or at `return`; `a = f(..)` frees the old value after evaluating the right side; call-result
temporaries are freed at the end of the statement/condition (only on the executed path).
"""
import dataclasses
import re
from pathlib import Path
from vast import *
import codegen
from codegen import show
from llvm_inline import llvm_inline
from typeinfo import TypeTable, is_user_type, is_array, uses_array_type, uses_str  # G2 B2: uses_str
from typeinfo import field_place
from check import array_resize_params, array_resize_args, same_place
from typeinfo import is_collection, contains_collection, encode, is_scalar_vec, is_vec_slice_arg, is_owned_result
from foreign import CATALOG, CATALOG_CONTRACT_ONLY, first_g6_use, _owned_result, NET_OBSERVERS, ASYNC_OBSERVERS  # G6 A3, G6b, G10 net A3
import foreign_shim  # G6 A3
import parallel_shim
from foreign import first_par
from storage import plan_storage, elem_size, alignment

G6_UNSUPPORTED_MSG = "G6 foreign calls and collections are not implemented by the llvm codegen layer yet"
G6_UNSUPPORTED_FIX = "avoid trusted rust fn, Vec, HashMap, String, take and move for now"
G6_BRIDGE_DECL = "@pruv_g6_bridge_v1 = external global i8\n"


def sym_prefix(pkg):
    """P10H: the prefix of every program-specific non-internal global symbol of the LLVM build (`pruv_abi_*`,
    `pruv_g6_v1_*`, `pruv_g6_bridge_v1`, the C adapter): `<pkg>__` for a program with a package name, so two programs
    link into one binary.  A program without a package name keeps the unprefixed names (it cannot be linked together
    with another program that defines the same symbols).  The program-independent runtime is never prefixed."""
    return pkg + "__" if pkg else ""

SIGNED_MIN128 = -170141183460469231731687303715884105728
BIT_LLVM = {n: llvm for n, (_rust, llvm) in codegen.BIT_BUILTINS.items()}   # P2: bit builtin -> LLVM instruction (from check.BIT_BUILTINS)
BIT_OP_LLVM = {"&": "and", "|": "or", "^": "xor", "<<": "shl", ">>": "ashr"}
EQR_LOOP_MAX = 4   # P17a: a dynamic-length eq_range below this many elements uses the inline loop, else memcmp
OPT = "{ i1, i64 }"
COPT = "{ i1, i128 }"       # Option/Result stored inside a contract aggregate (%pruv_c_*)
ARR = "{ ptr, i64, i64 }"   # P3: owned buffer inside the module: { data, len, cap }; cap = allocated element count

# G1 (docs/M2_G1_SPEC.md section 5): user struct/enum layout
#   %pruv_t_Name  struct: its fields in order ({} if none); enum: { i64 tag, every variant's payload fields in
#                variant order then field order }.  Inactive payload is always zeroinitializer-derived.
#   %pruv_c_Name  same shape for requires evaluation: integer leaves are i128, Option/Result are { i1, i128 }.

# P3: pruv_alloc / pruv_copy (and pruv_alloc_u8 / pruv_copy_v2 in DECLS_U8) are inline module copies of the
# runtime functions of the same name: len <= 0 gives NULL, a len above the runtime's bound panics with the same
# message, and the block comes from pruv_raw_alloc_v3 / pruv_raw_alloc_zeroed_v3 (runtime.c), so the constant
# layout and length checks fold away and only the allocator call is left.
# The allocator entry points (and pruv_free / pruv_free_v2) touch only allocator state the module cannot see, plus
# the freed block itself, so they carry memory(inaccessiblemem / argmem) and a call no longer clobbers the caller's
# loaded values. No willreturn: an allocation failure ends the process.
DECLS = """\
declare noalias nonnull ptr @pruv_raw_alloc_v3(i64, i64, i64) nounwind memory(inaccessiblemem: readwrite)
declare noalias nonnull ptr @pruv_raw_alloc_zeroed_v3(i64, i64, i64) nounwind memory(inaccessiblemem: readwrite)
@pruv_alloc_failed_msg = private unnamed_addr constant [18 x i8] c"allocation failed\\00"
define internal noalias ptr @pruv_alloc(i64 %n, i64 %v) alwaysinline nounwind {
entry:
  %e = icmp sle i64 %n, 0
  br i1 %e, label %none, label %chk
none:
  ret ptr null
chk:
  %big = icmp ugt i64 %n, 1152921504606846975
  br i1 %big, label %fail, label %ok
fail:
  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)
  unreachable
ok:
  %z = icmp eq i64 %v, 0
  br i1 %z, label %zero, label %fill
zero:
  %p = call ptr @pruv_raw_alloc_zeroed_v3(i64 %n, i64 8, i64 8)
  ret ptr %p
fill:
  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 8, i64 8)
  br label %loop
loop:
  %i = phi i64 [ 0, %fill ], [ %i1, %loop ]
  %g = getelementptr inbounds i64, ptr %q, i64 %i
  store i64 %v, ptr %g
  %i1 = add nuw nsw i64 %i, 1
  %d = icmp eq i64 %i1, %n
  br i1 %d, label %done, label %loop
done:
  ret ptr %q
}
define internal noalias ptr @pruv_copy(ptr nocapture readonly %s, i64 %n) alwaysinline nounwind {
entry:
  %e = icmp sle i64 %n, 0
  br i1 %e, label %none, label %chk
none:
  ret ptr null
chk:
  %big = icmp ugt i64 %n, 1152921504606846975
  br i1 %big, label %fail, label %ok
fail:
  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)
  unreachable
ok:
  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 8, i64 8)
  %b = shl nuw nsw i64 %n, 3
  call void @llvm.memcpy.p0.p0.i64(ptr %q, ptr %s, i64 %b, i1 false)
  ret ptr %q
}
declare void @llvm.memcpy.p0.p0.i64(ptr noalias nocapture writeonly, ptr noalias nocapture readonly, i64, i1 immarg)
declare void @llvm.memset.p0.i64(ptr nocapture writeonly, i8, i64, i1)
declare void @pruv_free(ptr nocapture, i64) nounwind memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @pruv_print_i64(i64) nounwind
declare void @pruv_print_bool(i64) nounwind
declare void @pruv_panic_oob(i64, i64) noreturn cold nounwind
declare void @pruv_panic_arith(ptr) noreturn cold nounwind
declare void @pruv_panic_requires(ptr, ptr) noreturn cold nounwind
declare void @pruv_panic_undefined(ptr) noreturn cold nounwind
declare { i64, i1 } @llvm.sadd.with.overflow.i64(i64, i64)
declare { i64, i1 } @llvm.ssub.with.overflow.i64(i64, i64)
declare { i64, i1 } @llvm.smul.with.overflow.i64(i64, i64)
declare { i128, i1 } @llvm.sadd.with.overflow.i128(i128, i128)
declare { i128, i1 } @llvm.ssub.with.overflow.i128(i128, i128)
declare { i128, i1 } @llvm.smul.with.overflow.i128(i128, i128)
define internal void @pruv_free_nn(ptr %p, i64 %n) alwaysinline nounwind {
entry:
  %z = icmp eq ptr %p, null
  br i1 %z, label %done, label %do
do:
  call void @pruv_free(ptr %p, i64 %n)
  br label %done
done:
  ret void
}
"""

# G2 A4 (docs/G2_A4_IMPL.md 2.9): only emitted when the program uses [u8]
DECLS_U8 = """\
define internal noalias ptr @pruv_alloc_u8(i64 %n, i64 %v) alwaysinline nounwind {
entry:
  %e = icmp sle i64 %n, 0
  br i1 %e, label %none, label %chk
none:
  ret ptr null
chk:
  %big = icmp ugt i64 %n, 1099511627776
  br i1 %big, label %fail, label %ok
fail:
  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)
  unreachable
ok:
  %z = icmp eq i64 %v, 0
  br i1 %z, label %zero, label %fill
zero:
  %p = call ptr @pruv_raw_alloc_zeroed_v3(i64 %n, i64 1, i64 1)
  ret ptr %p
fill:
  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 1, i64 1)
  %b = trunc i64 %v to i8
  call void @llvm.memset.p0.i64(ptr %q, i8 %b, i64 %n, i1 false)
  ret ptr %q
}
define internal noalias ptr @pruv_copy_v2(ptr nocapture readonly %s, i64 %n, i64 %sz, i64 %al) alwaysinline nounwind {
entry:
  %e = icmp sle i64 %n, 0
  br i1 %e, label %none, label %chk
none:
  ret ptr null
chk:
  %big = icmp ugt i64 %n, 1099511627776
  br i1 %big, label %fail, label %ok
fail:
  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)
  unreachable
ok:
  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 %sz, i64 %al)
  %b = mul nuw nsw i64 %n, %sz
  call void @llvm.memcpy.p0.p0.i64(ptr %q, ptr %s, i64 %b, i1 false)
  ret ptr %q
}
declare void @pruv_free_v2(ptr nocapture, i64, i64, i64) nounwind memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @pruv_panic_elem(i64, ptr) noreturn cold nounwind
define internal void @pruv_free_v2_nn(ptr %p, i64 %n, i64 %s, i64 %a) alwaysinline nounwind {
entry:
  %z = icmp eq ptr %p, null
  br i1 %z, label %done, label %do
do:
  call void @pruv_free_v2(ptr %p, i64 %n, i64 %s, i64 %a)
  br label %done
done:
  ret void
}
"""


# G2 A5 (docs/G2_A5_IMPL.md 4.2): only emitted when the program uses [bool]
DECLS_BOOL = "declare void @pruv_check_bool(ptr nocapture readonly, i64) nounwind\n"

# P3: only emitted when a tail-filled alloc is lowered (Gen.tail_fill): `alloc(n, 0)` whose first k elements the
# next loop overwrites zeroes only elements k..n (k clamped to 0..n); a large block still comes from the zeroed
# allocator, whose fresh pages are already zero.
DECLS_TAIL = """\
define internal noalias ptr @pruv_alloc_tail(i64 %n, i64 %k) alwaysinline nounwind {
entry:
  %e = icmp sle i64 %n, 0
  br i1 %e, label %none, label %chk
none:
  ret ptr null
chk:
  %big = icmp ugt i64 %n, 1152921504606846975
  br i1 %big, label %fail, label %ok
fail:
  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)
  unreachable
ok:
  %large = icmp ugt i64 %n, 16384
  br i1 %large, label %zero, label %part
zero:
  %p = call ptr @pruv_raw_alloc_zeroed_v3(i64 %n, i64 8, i64 8)
  ret ptr %p
part:
  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 8, i64 8)
  %k0 = call i64 @llvm.smax.i64(i64 %k, i64 0)
  %k1 = call i64 @llvm.smin.i64(i64 %k0, i64 %n)
  %g = getelementptr inbounds i64, ptr %q, i64 %k1
  %r = sub nuw nsw i64 %n, %k1
  %b = shl nuw nsw i64 %r, 3
  call void @llvm.memset.p0.i64(ptr %g, i8 0, i64 %b, i1 false)
  ret ptr %q
}
"""
DECLS_TAIL_U8 = """\
define internal noalias ptr @pruv_alloc_tail_u8(i64 %n, i64 %k) alwaysinline nounwind {
entry:
  %e = icmp sle i64 %n, 0
  br i1 %e, label %none, label %chk
none:
  ret ptr null
chk:
  %big = icmp ugt i64 %n, 1099511627776
  br i1 %big, label %fail, label %ok
fail:
  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)
  unreachable
ok:
  %large = icmp ugt i64 %n, 131072
  br i1 %large, label %zero, label %part
zero:
  %p = call ptr @pruv_raw_alloc_zeroed_v3(i64 %n, i64 1, i64 1)
  ret ptr %p
part:
  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 1, i64 1)
  %k0 = call i64 @llvm.smax.i64(i64 %k, i64 0)
  %k1 = call i64 @llvm.smin.i64(i64 %k0, i64 %n)
  %g = getelementptr inbounds i8, ptr %q, i64 %k1
  %r = sub nuw nsw i64 %n, %k1
  call void @llvm.memset.p0.i64(ptr %g, i8 0, i64 %r, i1 false)
  ret ptr %q
}
"""
DECLS_MINMAX = "declare i64 @llvm.smax.i64(i64, i64)\ndeclare i64 @llvm.smin.i64(i64, i64)\n"

# P05G: only emitted when resize is lowered.  resize(a, n, v) with m = len(a) <= n: one uninitialized block of n
# elements, one memcpy of the m-element prefix, then the n - m tail is set to v (a memset when v is 0 or the
# elements are bytes).  Every element is written before the block is returned.  n <= 0 gives NULL like pruv_alloc.
DECLS_RESIZE = """\
define internal noalias ptr @pruv_resize(ptr nocapture readonly %s, i64 %m, i64 %n, i64 %v) alwaysinline nounwind {
entry:
  %e = icmp sle i64 %n, 0
  br i1 %e, label %none, label %chk
none:
  ret ptr null
chk:
  %big = icmp ugt i64 %n, 1152921504606846975
  br i1 %big, label %fail, label %ok
fail:
  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)
  unreachable
ok:
  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 8, i64 8)
  %b = shl nuw nsw i64 %m, 3
  call void @llvm.memcpy.p0.p0.i64(ptr %q, ptr %s, i64 %b, i1 false)
  %t = getelementptr inbounds i64, ptr %q, i64 %m
  %r = sub nuw nsw i64 %n, %m
  %z = icmp eq i64 %v, 0
  br i1 %z, label %zero, label %fill
zero:
  %rb = shl nuw nsw i64 %r, 3
  call void @llvm.memset.p0.i64(ptr %t, i8 0, i64 %rb, i1 false)
  ret ptr %q
fill:
  %w = icmp eq i64 %r, 0
  br i1 %w, label %done, label %loop
loop:
  %i = phi i64 [ 0, %fill ], [ %i1, %loop ]
  %g = getelementptr inbounds i64, ptr %t, i64 %i
  store i64 %v, ptr %g
  %i1 = add nuw nsw i64 %i, 1
  %d = icmp eq i64 %i1, %r
  br i1 %d, label %done, label %loop
done:
  ret ptr %q
}
"""
DECLS_RESIZE_U8 = """\
define internal noalias ptr @pruv_resize_u8(ptr nocapture readonly %s, i64 %m, i64 %n, i64 %v) alwaysinline nounwind {
entry:
  %e = icmp sle i64 %n, 0
  br i1 %e, label %none, label %chk
none:
  ret ptr null
chk:
  %big = icmp ugt i64 %n, 1099511627776
  br i1 %big, label %fail, label %ok
fail:
  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)
  unreachable
ok:
  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 1, i64 1)
  call void @llvm.memcpy.p0.p0.i64(ptr %q, ptr %s, i64 %m, i1 false)
  %t = getelementptr inbounds i8, ptr %q, i64 %m
  %r = sub nuw nsw i64 %n, %m
  %b = trunc i64 %v to i8
  call void @llvm.memset.p0.i64(ptr %t, i8 %b, i64 %r, i1 false)
  ret ptr %q
}
"""
# P05G: resize of a narrow (u8/i8/u16/i16/u32/i32) storage layout, size/align are the element's constant byte width
# (1, 2 or 4) so the branches fold after inlining.  Same shape as pruv_resize: one uninitialized block, one memcpy
# of the prefix, the tail set once (a memset for 0 or byte elements, else one store loop).  n <= 2^40 is proved
# (narrow layouts exist only in verified builds); the check mirrors pruv_resize_u8.
DECLS_RESIZE_NARROW = """\
define internal noalias ptr @pruv_resize_narrow(ptr nocapture readonly %s, i64 %m, i64 %n, i64 %v, i64 %size, i64 %align) alwaysinline nounwind {
entry:
  %e = icmp sle i64 %n, 0
  br i1 %e, label %none, label %chk
none:
  ret ptr null
chk:
  %big = icmp ugt i64 %n, 1099511627776
  br i1 %big, label %fail, label %ok
fail:
  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)
  unreachable
ok:
  %q = call ptr @pruv_raw_alloc_v3(i64 %n, i64 %size, i64 %align)
  %b = mul nuw nsw i64 %m, %size
  call void @llvm.memcpy.p0.p0.i64(ptr %q, ptr %s, i64 %b, i1 false)
  %t = getelementptr inbounds i8, ptr %q, i64 %b
  %r = sub nuw nsw i64 %n, %m
  %z = icmp eq i64 %v, 0
  %one = icmp eq i64 %size, 1
  %set = or i1 %z, %one
  br i1 %set, label %bytes, label %wide
bytes:
  %rb = mul nuw nsw i64 %r, %size
  %vb = trunc i64 %v to i8
  call void @llvm.memset.p0.i64(ptr %t, i8 %vb, i64 %rb, i1 false)
  ret ptr %q
wide:
  %w = icmp eq i64 %r, 0
  br i1 %w, label %done, label %pick
pick:
  %two = icmp eq i64 %size, 2
  %v16 = trunc i64 %v to i16
  %v32 = trunc i64 %v to i32
  br i1 %two, label %loop16, label %loop32
loop16:
  %i = phi i64 [ 0, %pick ], [ %i1, %loop16 ]
  %g = getelementptr inbounds i16, ptr %t, i64 %i
  store i16 %v16, ptr %g
  %i1 = add nuw nsw i64 %i, 1
  %d = icmp eq i64 %i1, %r
  br i1 %d, label %done, label %loop16
loop32:
  %j = phi i64 [ 0, %pick ], [ %j1, %loop32 ]
  %h = getelementptr inbounds i32, ptr %t, i64 %j
  store i32 %v32, ptr %h
  %j1 = add nuw nsw i64 %j, 1
  %c = icmp eq i64 %j1, %r
  br i1 %c, label %done, label %loop32
done:
  ret ptr %q
}
"""
# LG32 (G-DB52-4): only emitted when `x = resize(x, n, v)` is lowered in place.  The block s (allocated count c, live
# count m <= n) grows through pruv_regrow_v2 (realloc, so the old block is never live next to the new one) when n > c,
# else it is kept; then the tail [m, n) is set to v exactly like pruv_resize / pruv_resize_u8.  n <= 0 (so the array is
# empty) returns s unchanged.
DECLS_REGROW = """\
define internal ptr @pruv_regrow(ptr %s, i64 %c, i64 %m, i64 %n, i64 %v) alwaysinline nounwind {
entry:
  %e = icmp sle i64 %n, 0
  br i1 %e, label %none, label %chk
none:
  ret ptr %s
chk:
  %big = icmp ugt i64 %n, 1152921504606846975
  br i1 %big, label %fail, label %ok
fail:
  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)
  unreachable
ok:
  %gr = icmp sgt i64 %n, %c
  br i1 %gr, label %grow, label %have
grow:
  %p = call ptr @pruv_regrow_v2(ptr %s, i64 %c, i64 %n, i64 8, i64 8)
  br label %have
have:
  %q = phi ptr [ %s, %ok ], [ %p, %grow ]
  %t = getelementptr inbounds i64, ptr %q, i64 %m
  %r = sub nuw nsw i64 %n, %m
  %z = icmp eq i64 %v, 0
  br i1 %z, label %zero, label %fill
zero:
  %rb = shl nuw nsw i64 %r, 3
  call void @llvm.memset.p0.i64(ptr %t, i8 0, i64 %rb, i1 false)
  ret ptr %q
fill:
  %w = icmp eq i64 %r, 0
  br i1 %w, label %done, label %loop
loop:
  %i = phi i64 [ 0, %fill ], [ %i1, %loop ]
  %g = getelementptr inbounds i64, ptr %t, i64 %i
  store i64 %v, ptr %g
  %i1 = add nuw nsw i64 %i, 1
  %d = icmp eq i64 %i1, %r
  br i1 %d, label %done, label %loop
done:
  ret ptr %q
}
"""
DECLS_REGROW_U8 = """\
define internal ptr @pruv_regrow_u8(ptr %s, i64 %c, i64 %m, i64 %n, i64 %v) alwaysinline nounwind {
entry:
  %e = icmp sle i64 %n, 0
  br i1 %e, label %none, label %chk
none:
  ret ptr %s
chk:
  %big = icmp ugt i64 %n, 1099511627776
  br i1 %big, label %fail, label %ok
fail:
  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)
  unreachable
ok:
  %gr = icmp sgt i64 %n, %c
  br i1 %gr, label %grow, label %have
grow:
  %p = call ptr @pruv_regrow_v2(ptr %s, i64 %c, i64 %n, i64 1, i64 1)
  br label %have
have:
  %q = phi ptr [ %s, %ok ], [ %p, %grow ]
  %t = getelementptr inbounds i8, ptr %q, i64 %m
  %r = sub nuw nsw i64 %n, %m
  %b = trunc i64 %v to i8
  call void @llvm.memset.p0.i64(ptr %t, i8 %b, i64 %r, i1 false)
  ret ptr %q
}
"""
# LG34 (G-LG32-3): the regrow of a narrow (u8/i8/u16/i16/u32/i32) storage layout; size/align are the element's
# constant byte width (1, 2 or 4) so the branches fold after inlining.  Same growth as pruv_regrow, same tail fill as
# pruv_resize_narrow (a memset for 0 or byte elements, else one store loop).
DECLS_REGROW_NARROW = """\
define internal ptr @pruv_regrow_narrow(ptr %s, i64 %c, i64 %m, i64 %n, i64 %v, i64 %size, i64 %align) alwaysinline nounwind {
entry:
  %e = icmp sle i64 %n, 0
  br i1 %e, label %none, label %chk
none:
  ret ptr %s
chk:
  %big = icmp ugt i64 %n, 1099511627776
  br i1 %big, label %fail, label %ok
fail:
  call void @pruv_panic_arith(ptr @pruv_alloc_failed_msg)
  unreachable
ok:
  %gr = icmp sgt i64 %n, %c
  br i1 %gr, label %grow, label %have
grow:
  %p = call ptr @pruv_regrow_v2(ptr %s, i64 %c, i64 %n, i64 %size, i64 %align)
  br label %have
have:
  %q = phi ptr [ %s, %ok ], [ %p, %grow ]
  %b = mul nuw nsw i64 %m, %size
  %t = getelementptr inbounds i8, ptr %q, i64 %b
  %r = sub nuw nsw i64 %n, %m
  %z = icmp eq i64 %v, 0
  %one = icmp eq i64 %size, 1
  %set = or i1 %z, %one
  br i1 %set, label %bytes, label %wide
bytes:
  %rb = mul nuw nsw i64 %r, %size
  %vb = trunc i64 %v to i8
  call void @llvm.memset.p0.i64(ptr %t, i8 %vb, i64 %rb, i1 false)
  ret ptr %q
wide:
  %w = icmp eq i64 %r, 0
  br i1 %w, label %done, label %pick
pick:
  %two = icmp eq i64 %size, 2
  %v16 = trunc i64 %v to i16
  %v32 = trunc i64 %v to i32
  br i1 %two, label %loop16, label %loop32
loop16:
  %i = phi i64 [ 0, %pick ], [ %i1, %loop16 ]
  %g = getelementptr inbounds i16, ptr %t, i64 %i
  store i16 %v16, ptr %g
  %i1 = add nuw nsw i64 %i, 1
  %d = icmp eq i64 %i1, %r
  br i1 %d, label %done, label %loop16
loop32:
  %j = phi i64 [ 0, %pick ], [ %j1, %loop32 ]
  %h = getelementptr inbounds i32, ptr %t, i64 %j
  store i32 %v32, ptr %h
  %j1 = add nuw nsw i64 %j, 1
  %k = icmp eq i64 %j1, %r
  br i1 %k, label %done, label %loop32
done:
  ret ptr %q
}
"""
DECL_REGROW_V2 = "declare ptr @pruv_regrow_v2(ptr, i64, i64, i64, i64) nounwind\n"

# G9 A3: only emitted when the program has a recursive (boxed) edge.  The marker global is kept alive through
# @llvm.used, so linking a runtime without pruv_runtime_nodes_v1 fails even when no node helper survives inlining.
DECLS_NODE = ("declare noalias ptr @pruv_node_alloc_v1(i64, i64) nounwind\n"
              "declare void @pruv_node_free_v1(ptr nocapture, i64, i64) nounwind\n"
              "declare void @pruv_runtime_nodes_v1() nounwind\n"
              "@pruv_nodes_marker = internal global ptr @pruv_runtime_nodes_v1\n"
              '@llvm.used = appending global [1 x ptr] [ptr @pruv_nodes_marker], section "llvm.metadata"\n')

# G2 B2 (docs/G2_B2_IMPL.md 2.7): only emitted when the program uses str
DECLS_STR = ("declare i32 @pruv_str_boundary(ptr nocapture readonly, i64, i64) nounwind willreturn memory(argmem: read)\n"
             "declare noalias ptr @pruv_substr(ptr nocapture readonly, i64, i64, i64) nounwind\n"
             "declare void @pruv_check_utf8(ptr nocapture readonly, i64) nounwind\n")


def is_arr(t):  # G2 B2: array-like type (arrays and str share the { ptr, i64 } representation)
    return is_array(t) or t.name == "str"


def elem_ty(t):
    """element info carried in V/Slot.ty of an array: ARRAY_U8 for [u8], ARRAY_BOOL for [bool], STR for str,
    None for logical [i64]; V/Slot.layout separately carries its storage width"""
    if t is not None and t.name == "array_u8":
        return ARRAY_U8
    if t is not None and t.name == "array_bool":  # G2 A5
        return ARRAY_BOOL
    if t is not None and t.name == "str":  # G2 B2
        return STR
    return None


def is_u8(v):
    return v.ty is not None and v.ty.name == "array_u8"


def is_bytes(v):  # G2 A5/B2: byte-sized element arrays ([u8], [bool] or str)
    return v.ty is not None and v.ty.name in ("array_u8", "array_bool", "str")


def is_strv(v):  # G2 B2
    return v.ty is not None and v.ty.name == "str"


def is_boolarr(v):  # G2 A5
    return v.ty is not None and v.ty.name == "array_bool"


def mangle(n):
    return re.sub(r"[^A-Za-z0-9_]", lambda m: "$%04x" % ord(m.group()), n)


# G3 A4 (docs/G3_SPEC.md 5.4/5.5): the back end consumes the concrete specialization plan (mono.py), so no type
# parameter ever reaches it.  New built-in applications Option<X> / Result<X> with X != i64 are ordinary aggregates
# (V.k == "agg", V.ty = the Option/Result type with its argument):
#   Option<X>  { i1 present, L(X) payload }                 contract form { i1, Lc(X) }
#   Result<X>  { i1 is_ok, L(X) ok_payload, i64 error }     contract form { i1, Lc(X), i128 }
# Unused payload storage is zero (every constructor starts from zeroinitializer, deserialization masks inactive
# words); equality and selectors look only at the active payload.  Legacy Option<i64>/Result<i64> are unchanged.
G3_TYPE_PREFIX = "pruv_g3_t_"
DECLS_REPR = "declare void @pruv_panic_repr(i64, ptr) noreturn cold nounwind\n"
# mutable host entries: byte-range overlap test of two argument regions (negative length or address arithmetic
# overflow counts as an overlap; an empty region never overlaps)
DECLS_OVERLAP = """\
declare { i64, i1 } @llvm.umul.with.overflow.i64(i64, i64)
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
"""


def gopt(t):
    """True for a new built-in application Option<X> / Result<X> (X != i64); False for the legacy constants"""
    return t.name in ("option", "result") and bool(t.args) and t.owner is None


def is_agg(t):
    """user struct/enum (incl. generated generic instances) or a new Option/Result application"""
    return is_user_type(t) or gopt(t)


def is_new_app(t):
    """types whose external word form follows the G3 rules (tag normalization, inactive words zero, enum check)"""
    return gopt(t) or (is_user_type(t) and t.name.startswith(G3_TYPE_PREFIX))


def lty(t):
    """internal SSA type of a Pruv type"""
    if is_collection(t):  # G6 A3: opaque handle
        return "ptr"
    if gopt(t):
        return st(t)
    r = {"i64": "i64", "bool": "i1", "unit": "void", "array": ARR, "array_u8": ARR, "array_bool": ARR,
         "str": ARR, "option": OPT, "result": OPT}.get(t.name)
    return r if r is not None else "%%pruv_t_%s" % t.name


def vkind(t):
    if is_collection(t):  # G6 A3
        return "coll"
    if gopt(t):
        return "agg"
    return {"i64": "i64", "bool": "i1", "unit": "unit", "array": "arr", "array_u8": "arr", "array_bool": "arr",
            "str": "arr", "option": "opt", "result": "opt"}.get(t.name, "agg")


def st(t, c=False):
    """storage type of a value of type t inside an aggregate (c: contract/i128 form)"""
    n = t.name
    if is_collection(t):    # LG12 E2: the error slot of Result<T, String> holds the String handle
        return "ptr"
    if gopt(t):
        x = st(t.args[0], c)
        if n == "option":
            return "{ i1, %s }" % x
        return "{ i1, %s, %s }" % (x, st(result_error(t), c))   # LG12: L(E) error slot (i64 for Result<X>)
    if n == "i64":
        return "i128" if c else "i64"
    if n == "bool":
        return "i1"
    if n in ("option", "result"):
        return COPT if c else OPT
    if n in ("array", "array_u8", "array_bool", "str"):  # G9 A3: buffer fields keep the { ptr, i64 } form
        return ARR
    return "%%pruv_%s_%s" % ("c" if c else "t", n)


def payload_index(tt, ty, variant, i):
    """storage index of field i of `variant` inside the flattened enum %pruv_t_/%pruv_c_ type"""
    # the variant -> base offset map is computed once per (type table, type name): rescanning the variants on every
    # call made helper generation quadratic in the number of variants
    memo = tt.__dict__.setdefault("_payload_memo", {})
    offs = memo.get(ty.name)
    if offs is None:
        offs, base = {}, 1
        for v in tt.decl(ty).variants:
            offs[v.name] = base
            base += len(v.fields)
        memo[ty.name] = offs
    if variant not in offs:
        raise KeyError(variant)
    return offs[variant] + i


def words(tt, ty):
    """W(T): number of i64 words of the external (pruv_abi_*) form.  Memoized per type table (a nested generic
    value would otherwise recompute every subtree: quadratic)."""
    memo = tt.__dict__.setdefault("_words_memo", {})
    r = memo.get(ty)
    if r is not None:
        return r
    n = ty.name
    if n in ("i64", "bool"):
        r = 1
    elif gopt(ty):  # G3: 1 + W(X) (+ 1 for the Result error word)
        r = 1 + words(tt, ty.args[0]) + (words(tt, result_error(ty)) if n == "result" else 0)
    elif n in ("option", "result"):
        r = 2
    else:
        d = tt.decl(ty)
        if isinstance(d, StructDecl):
            r = max(1, sum(words(tt, f.type) for f in d.fields))
        else:
            r = 1 + sum(words(tt, f.type) for v in d.variants for f in v.fields)
    memo[ty] = r
    return r


def fld_label(d, vname, fname):
    """G9 field label as in prog.g9["boxed"]: "Struct.field" or "Enum::Variant.field" """
    return "%s.%s" % (d.name, fname) if vname is None else "%s::%s.%s" % (d.name, vname, fname)


def new_tt(prog):
    """type table of a program: SCC-condensed declaration order for G9 programs (identical to TypeTable otherwise),
    plus the G9 metadata sets tt.g9_ext (extended declaration names) and tt.g9_boxed (boxed field labels)"""
    tt = codegen.make_type_table(prog)
    tt.g9_ext, tt.g9_boxed = codegen.g9_sets(prog)
    return tt


def is_ext_t(tt, t):
    """t is an extended user type (G9): recursive, or holding a buffer or an extended type"""
    if t is not None and is_owned_result(t):
        # LG12 E2: Result<T, String> is an owned aggregate { i1, L(T), ptr }: it travels through output storage / ptr
        # parameters like an extended value and has no host ABI wrapper; its String handle is null on the Ok side
        return True
    return bool(tt.g9_ext) and t is not None and t.owner is None and not t.args and t.name in tt.g9_ext


def type_defs(tt):
    out = []
    ext, boxed = tt.g9_ext, tt.g9_boxed
    for c in (False, True):
        for d in tt.order:
            if c and d.name in ext:     # G9: an extended value has no i128 contract mirror
                continue

            def fs_of(fields, vname):
                # a boxed (recursive SCC) field is an opaque ptr to an independently allocated node
                return ["ptr" if fld_label(d, vname, f.name) in boxed else st(f.type, c) for f in fields]
            if isinstance(d, StructDecl):
                fs = fs_of(d.fields, None)
            else:
                fs = ["i64"]
                for v in d.variants:
                    fs += fs_of(v.fields, v.name)
            out.append("%%pruv_%s_%s = type %s" % ("c" if c else "t", d.name, "{ %s }" % ", ".join(fs) if fs else "{}"))
    return "\n".join(out) + ("\n" if out else "")


def type_has_field(prog, name):
    """G9: some struct field / enum payload has the internal type `name` (array_u8, array_bool, str)"""
    for d in prog.types:
        fields = d.fields if isinstance(d, StructDecl) else [f for v in d.variants for f in v.fields]
        if any(f.type.name == name for f in fields):
            return True
    return False


def storage_ty(v):
    return "i8" if is_bytes(v) else "i" + v.layout[1:]


class V:
    """an SSA value: k in i64 | i1 | i128 | arr (a=ptr, b=len) | opt (a=tag i1, b=payload) | unit |
    agg (a=aggregate SSA of a user struct/enum, ty=its Type; %pruv_t_* in code, %pruv_c_* in requires) |
    coll (G6: a=ptr, an opaque handle of a Vec/HashMap/String box, ty=its Type) |
    ref (G9 contract evaluation: a=ptr to the REAL storage of an extended value, ty=its Type).
    p (G9, agg only): address of storage that holds this value when known (a parameter, a local slot, a boxed
    node, a call-result slot); lets a read argument pass the pointer instead of spilling a copy."""
    __slots__ = ("k", "a", "b", "ty", "p", "layout", "rng", "mv", "c")

    def __init__(s, k, a=None, b=None, ty=None, p=None, layout="i64", rng=None, c=None):
        s.k, s.a, s.b, s.ty, s.p = k, a, b, ty, p
        s.layout = layout
        s.rng = rng
        s.mv = None     # P3: (owner slot, variant, storage index) of a payload binder of a match on an owned local
        s.c = c         # P3 (arr only): allocated element count (cap) SSA; None means cap == len (b)


def capof(v):
    """P3: the allocated element count of an owned array value (the size argument of every free)"""
    return v.c if v.c is not None else v.b


class Slot:
    __slots__ = ("k", "a", "b", "ty", "layout", "rng", "c")

    def __init__(s, k, a, b=None, ty=None):
        s.k, s.a, s.b, s.ty = k, a, b, ty
        s.layout = "i64"
        s.rng = None
        s.c = None      # P3 (arr only): storage of the cap


class Mod:
    def __init__(s, tt=None):
        s.tt = tt
        s.array_resizes = set()
        s.strs = {}
        s.order = []
        s.uses_repr = False     # G3: the wrappers reject an invalid enum ordinal through pruv_panic_repr
        s.par_types = []
        s.par_trampolines = []
        s.sym_pre = ""          # P10H: `<pkg>__` (or "") in front of every program-specific global symbol (sym_prefix)
        s.g6_decls = {}         # G6 A3: shim symbol -> declare line
        s.g6_v2 = False         # G6 A3: string_to_str needs the v2 copy/free declarations
        s.g6_take = False       # P3 M3: some string_to_str takes the String buffer (pruv_g6_v1_string_take)
        s.g6_unbox = set()      # P3 M5: trusted String functions lowered through the unboxed adapter <f>__str
        s.g6_bridge = False     # G6 A3: some function references @pruv_g6_bridge_v1
        s.uses_utf8_valid = False
        s.uses_utf8_panic = False
        s.bswap = set()         # P2 batch 4: widths of the llvm.bswap declarations the byte-word accesses used
        s.shrink = False        # P3: some external array-return wrapper normalizes cap to len with @pruv_shrink_v2
        s.overlap = False       # mutable host entries: some wrapper calls @pruv_overlap (DECLS_OVERLAP is emitted)
        s.need_size = set()     # G9 A3: types whose runtime-contract size / equality helpers are needed
        s.need_eq = set()
        s.stack = None          # P3-S1: StackPlan (set by gen_llvm)
        s.fill = False          # P3-S1: some stack array calls @pruv_fill_v2
        s.tail = set()          # P3: tail-filled alloc helpers used ("i64" -> pruv_alloc_tail, "u8" -> _u8)
        s.memmove = False
        s.memcmp = False        # P17a: some eq_range lowers to a memcmp call (declare i32 @memcmp)
        s.resize = set()        # P05G: resize helpers used ("i64" -> pruv_resize, "u8" -> pruv_resize_u8, "narrow")
        s.range_nodes = {}      # P3 batch 2: `iW a, iW b` body -> !N number (first-use order)
        s.explicit = False      # G4 A3: explicit visibility mode (set by gen_llvm); Pruv calls use the Rust-backend routing

    def string(s, text):
        if text not in s.strs:
            s.strs[text] = "@.str.%d" % len(s.order)
            s.order.append(text)
        return s.strs[text]

    def render_strings(s):
        out = []
        for text in s.order:
            b = text.encode("utf-8")
            body = "".join(chr(c) if 32 <= c < 127 and c not in (34, 92) else "\\%02X" % c for c in b)
            out.append('%s = private unnamed_addr constant [%d x i8] c"%s\\00"' % (s.strs[text], len(b) + 1, body))
        return "\n".join(out) + ("\n" if out else "")


def ir_ret(tt, f):
    """internal LLVM result type: an extended (G9) aggregate result goes through explicit output storage"""
    return "void" if is_ext_t(tt, f.ret) else lty(f.ret)


def has_inout(f):
    """G5 A3: some parameter has the inout mode (a mutable borrow of caller storage)"""
    return any(p.mode == "inout" for p in f.params)


def has_take_array(f):
    return any(p.take and is_array(p.type) for p in f.params)


def is_internal_fn(f, tt):
    """G9: a function with an extended type in its signature has no raw ABI wrapper and no binding;
    G5 A3: neither has a function with an inout parameter (one host-eligibility rule)"""
    return has_ext_sig(f, tt) or has_inout(f) or has_take_array(f)


def has_ext_sig(f, tt):
    """G9: an extended (boxed/aggregate-with-buffer) type occurs in the signature (no wrapper regardless of mode)"""
    return any(is_ext_t(tt, p.type) for p in f.params) or is_ext_t(tt, f.ret)


def exports_abi(f, tt, explicit):
    """the common export rule of the raw wrapper, the typed variant, the Rust binding and the public host wrapper:
    not foreign / specialization / collection-bearing / extended, explicit mode => host entry, and an inout parameter
    is exported only from an explicit-mode host entry (mutable host entries; legacy inout fns stay internal)"""
    if (f.foreign is not None or getattr(f, "private", False) or _is_coll_fn(f) or has_ext_sig(f, tt)
            or has_take_array(f)):
        return False
    if explicit and not f.host_entry:
        return False
    if has_inout(f) and not (explicit and f.host_entry):
        return False
    return True


def inout_slot_param(p):
    return p.mode == "inout" and (is_agg(p.type) or p.type in (I64, BOOL) or getattr(p, "resizable", False))


def ir_params(f, tt=None):
    """internal parameter list [(type, name)].  G9: an extended result is the leading `ptr %out` (output storage);
    an extended parameter is `ptr %p.NAME` to valid caller-owned storage (see sig_text for its attributes)."""
    out = []
    if tt is not None and is_ext_t(tt, f.ret):
        out.append(("ptr", "%out"))
    for p in f.params:
        if (tt is not None and is_ext_t(tt, p.type)) or inout_slot_param(p):
            out.append(("ptr", "%%p.%s" % mangle(p.name)))
            continue
        n = mangle(p.name)
        if is_arr(p.type):
            out += [("ptr", "%%p.%s.a" % n), ("i64", "%%p.%s.n" % n)]
            if p.take:
                out.append(("i64", "%%p.%s.c" % n))
        elif p.type == BOOL:
            out.append(("i1", "%%p.%s" % n))
        elif p.type == I64:
            out.append(("i64", "%%p.%s" % n))
        elif is_collection(p.type):  # G6 A3: plain opaque handle %p.NAME.h (no readonly/noalias attributes)
            out.append(("ptr", "%%p.%s.h" % n))
        elif is_agg(p.type):
            out.append((lty(p.type), "%%p.%s" % n))
        else:
            out.append((OPT, "%%p.%s" % n))
    return out


def sig_text(params, f):
    """Borrowed arrays and aggregates are readonly unless inout. Take arrays transfer the data pointer."""
    mut = {"%%p.%s%s" % (mangle(p.name), ".a" if is_arr(p.type) and not inout_slot_param(p) else "")
           for p in f.params if p.mode == "inout"}
    take = {"%%p.%s.a" % mangle(p.name) for p in f.params if p.take and is_array(p.type)}
    take |= {"%%p.%s" % mangle(p.name) for p in f.params if p.take and is_agg(p.type)}   # LG9: owned record storage
    return ", ".join("%s %s" % (("ptr noalias nocapture" if n in mut else "ptr noalias nocapture readonly")
                                if t == "ptr" and n != "%out" and not n.endswith(".h") and n not in take else t, n)
                     for t, n in params)


class FnGen:
    def __init__(s, m, funcs, f, verified):
        s.m, s.funcs, s.f, s.verified = m, funcs, f, verified
        s.cost = not verified and f.cost is not None
        s.lines = []
        s.allocas = []
        s.nt = s.nl = s.nslot = s.nq = 0
        s.cur = None
        s.dead = False
        s.scopes = []
        s.temps = []
        s.ret_cand = {}     # P3: return-movable names -> owner slot (see ret_movable)
        s.ret_moved = {}    # P3: owner slot address -> None (whole value moved) or {storage index of a moved field}
        s.frames = []       # P3: enclosing blocks being lowered: [names declared so far, names named later, loop body,
                            # LG34: the next statement is a return]
        s.moving = set()    # P3: names the copy of the let / assignment being lowered moves (move_src)
        s.ret_take = set()  # P3 M3: String locals whose string_to_str in the return result position takes the buffer
        s.take_let = set()  # P3 M3: String local the let / assignment being lowered takes (take_src)
        s.unbox_st = None   # P3 M5: the let being lowered binds an unboxed String local (unbox_let)
        s.tail_st = None    # P3: the let being lowered is a tail-filled alloc (tail_fill); s.tail_k is its bound (a Var)
        s.tail_k = None
        s.cenv = {}
        s.cregions = []     # G9: constructor temporaries of the contract expression being evaluated, per region
        s.tt = m.tt
        s.bridge = False    # G6 A3: this function references @pruv_g6_bridge_v1
        s.resize_args = array_resize_args(f, m.array_resizes) if f is not None and m.array_resizes else set()

    # ---------- emission ----------
    def ins(s, text):
        if s.dead:
            raise Exception("llvmgen: internal error: instruction after terminator: %s" % text)
        s.lines.append("  " + text)

    def term(s, text):
        s.ins(text)
        s.dead = True

    def tmp(s):
        s.nt += 1
        return "%%t%d" % s.nt

    def op(s, text):
        r = s.tmp()
        s.ins("%s = %s" % (r, text))
        return r

    def label(s, hint):
        s.nl += 1
        return "%s.%d" % (hint, s.nl)

    def place(s, lbl):
        s.lines.append(lbl + ":")
        s.cur = lbl
        s.dead = False

    def br(s, l):
        s.term("br label %%%s" % l)

    def condbr(s, c, a, b):
        s.term("br i1 %s, label %%%s, label %%%s" % (c, a, b))

    def guard(s, cond, failfn, fail_when):
        """continue only when cond != fail_when; else run failfn (a noreturn call) in a fail block"""
        ok, bad = s.label("ok"), s.label("fail")
        if fail_when:
            s.condbr(cond, bad, ok)
        else:
            s.condbr(cond, ok, bad)
        s.place(bad)
        failfn()
        s.term("unreachable")
        s.place(ok)

    def panic1(s, fn, msg):
        return lambda: s.ins("call void @%s(ptr %s)" % (fn, s.m.string(msg)))

    def panic_elem(s, x):
        return lambda: s.ins("call void @pruv_panic_elem(i64 %s, ptr %s)" % (x, s.m.string("u8 0..=255")))

    def render(s, header):
        out = [header + " {", "entry:"] + ["  " + a for a in s.allocas] + ["  br label %start"]
        lines = s.lines
        if s.bridge:
            # G6 A3: a (volatile) read of the shim's bridge marker keeps the reference alive: linking without the
            # shim fails.  lines[0] is the `start:` label.
            lines = lines[:1] + ["  %g6.bridge = load volatile i8, ptr @" + s.m.sym_pre + "pruv_g6_bridge_v1"] + lines[1:]
            s.m.g6_bridge = True
        out += lines
        out.append("}")
        return "\n".join(out) + "\n"

    # ---------- slots / scopes ----------
    def new_slot(s, name, ty, prefix="l", packed=False):
        kind = vkind(ty)
        base = "%%%s%d.%s" % (prefix, s.nslot, mangle(name))
        s.nslot += 1
        a, b = base + ".a", base + ".b"
        if kind == "i64":
            s.allocas.append("%s = alloca i64" % a)
            return Slot("i64", a)
        if kind == "i1":
            s.allocas.append("%s = alloca i1" % a)
            return Slot("i1", a)
        if kind == "coll":  # G6 A3: owner slot, null-initialized
            s.allocas.append("%s = alloca ptr" % a)
            s.allocas.append("store ptr null, ptr %s" % a)
            return Slot("coll", a, None, ty)
        if kind == "arr":
            if packed:
                s.allocas.append("%s = alloca %s" % (a, ARR))
                return s.array_slot(a, ty)
            s.allocas.append("%s = alloca ptr" % a)
            s.allocas.append("%s = alloca i64" % b)
            sl = Slot("arr", a, b, elem_ty(ty))
            sl.c = base + ".c"      # P3: the cap lives next to the len
            s.allocas.append("%s = alloca i64" % sl.c)
            return sl
        if kind == "agg":
            s.allocas.append("%s = alloca %s" % (a, lty(ty)))
            return Slot("agg", a, None, ty)
        s.allocas.append("%s = alloca i1" % a)
        s.allocas.append("%s = alloca i64" % b)
        return Slot("opt", a, b)

    def array_slot(s, ptr, ty):
        sl = Slot("arr", ptr, g9_gep(s, ARR, ptr, 1), elem_ty(ty))
        sl.c = g9_gep(s, ARR, ptr, 2)
        return sl

    def load(s, sl):
        if sl.k in ("i64", "i1"):
            return V(sl.k, s.op("load %s, ptr %s" % (sl.k, sl.a)))
        if sl.k == "agg":
            return V("agg", s.op("load %s, ptr %s" % (lty(sl.ty), sl.a)), None, sl.ty, sl.a)
        if sl.k == "coll":
            return V("coll", s.op("load ptr, ptr %s" % sl.a), None, sl.ty)
        if sl.k == "arr":
            p = s.op("load ptr, ptr %s" % sl.a)
            n = s.op("load i64, ptr %s" % sl.b)
            c = s.op("load i64, ptr %s" % sl.c)
            return V("arr", p, n, sl.ty, layout=sl.layout, rng=sl.rng, c=c)
        t = s.op("load i1, ptr %s" % sl.a)
        v = s.op("load i64, ptr %s" % sl.b)
        return V("opt", t, v)

    def store(s, sl, v):
        if sl.k in ("i64", "i1"):
            s.ins("store %s %s, ptr %s" % (sl.k, v.a, sl.a))
        elif sl.k == "agg":
            s.ins("store %s %s, ptr %s" % (lty(sl.ty), v.a, sl.a))
        elif sl.k == "coll":
            s.ins("store ptr %s, ptr %s" % (v.a, sl.a))
        elif sl.k == "arr":
            sl.layout = v.layout
            sl.rng = v.rng
            s.ins("store ptr %s, ptr %s" % (v.a, sl.a))
            s.ins("store i64 %s, ptr %s" % (v.b, sl.b))
            s.ins("store i64 %s, ptr %s" % (capof(v), sl.c))
        else:
            s.ins("store i1 %s, ptr %s" % (v.a, sl.a))
            s.ins("store i64 %s, ptr %s" % (v.b, sl.b))

    def lookup(s, name):
        for sc in reversed(s.scopes):
            if name in sc["names"]:
                return sc["names"][name]
        raise Exception("llvmgen: unknown variable %s" % name)

    def push_scope(s):
        s.scopes.append({"names": {}, "arrays": []})

    # ---------- G9 A3: extended values (ownership helpers) ----------
    def is_ext(s, t):
        return is_ext_t(s.tt, t)

    def fb(s, ty, variant, fname):
        """the field `fname` (of `variant` for an enum) of the user type ty is a boxed recursive edge"""
        return bool(s.tt.g9_boxed) and is_user_type(ty) and fld_label(s.tt.decl(ty), variant, fname) in s.tt.g9_boxed

    def spill(s, v):
        """address of storage that holds the aggregate v: its known address, else a fresh entry-block copy"""
        if v.p is not None:
            return v.p
        base = "%%sp%d" % s.nslot
        s.nslot += 1
        s.allocas.append("%s = alloca %s" % (base, lty(v.ty)))
        s.ins("store %s %s, ptr %s" % (lty(v.ty), v.a, base))
        return base

    def new_out(s, ty):
        """fresh entry-block output storage for an aggregate result"""
        base = "%%o%d" % s.nslot
        s.nslot += 1
        s.allocas.append("%s = alloca %s" % (base, lty(ty)))
        return base

    def fresh(s, v, owned):
        """a fresh extended value: owned by the destination, or a temporary dropped at the end of the statement"""
        if not owned:
            s.temps[-1].append(v)
        return v

    def clone_val(s, v):
        """deep copy of the extended value v (a view) into new owned storage"""
        src = s.spill(v)
        dst = s.new_out(v.ty)
        s.ins("call void @pruv_clone__%s(ptr %s, ptr %s)" % (v.ty.name, src, dst))
        return V("agg", s.op("load %s, ptr %s" % (lty(v.ty), dst)), None, v.ty, dst)

    def drop_agg(s, v):
        if is_owned_result(v.ty):   # LG12 E2
            s.drop_oresult(v.ty, s.spill(v))
            return
        s.ins("call void @pruv_drop__%s(ptr %s)" % (v.ty.name, s.spill(v)))

    def view_ptr(s, e):
        """address of the storage of an extended expression in a READ position (argument): a local slot or a
        parameter is passed as is, a fresh temporary lives (registered in s.temps) until the end of the statement"""
        if isinstance(e, Var):
            kind, x = s.lookup(e.name)
            if kind == "slot":
                return x.a
            return s.spill(x)
        return s.spill(s.ev(e))

    def inout_place_ptr(s, e):
        root, path = field_place(e)
        _, sl = s.lookup(root.name)
        return s.field_ptr_ty(sl, path)[0]

    def field_ptr_ty(s, sl, path):
        """LG4: (address of the field reached by `path` from the extended root slot sl, its declared type); a boxed
        intermediate edge is loaded"""
        ptr, ty = sl.a, sl.ty
        for field in path:
            i = s.tt.field_index(ty, field)
            ptr = g9_gep(s, lty(ty), ptr, i)
            if s.fb(ty, None, field):
                ptr = s.op("load ptr, ptr %s" % ptr)
            ty = s.tt.fields(ty)[i].type
        return ptr, ty

    def getf(s, base, ty, idx, fd, variant, c=False):
        """field value of the aggregate `base` (type ty) at storage index idx; a boxed field is a VIEW of the child
        node (a shallow load, never dropped through this value)"""
        if not c and s.fb(ty, variant, fd.name):
            x = s.op("extractvalue %s %s, %d" % (st(ty, c), base.a, idx))
            return V("agg", s.op("load %s, ptr %s" % (lty(fd.type), x)), None, fd.type, x)
        return s.get(base, ty, idx, fd.type, c)

    def putf(s, acc, ty, idx, v, fd, variant, c=False):
        """insertvalue of the owned field value v; a boxed field first moves into a new node"""
        if not c and s.fb(ty, variant, fd.name):
            nd = s.op("call ptr @pruv_box__%s(%s %s)" % (fd.type.name, lty(fd.type), v.a))
            return s.op("insertvalue %s %s, ptr %s, %d" % (st(ty, c), acc, nd, idx))
        return s.put(acc, ty, idx, v, fd.type, c)

    def assign_path(s, sl, path, v):
        """store the owned value v into the field path of the extended local slot sl (in place).  The old value of
        the last field is released (nodes: drop then free; buffers: free) after v is complete."""
        cur, cty = sl.a, sl.ty
        for k, fname in enumerate(path):
            i = s.tt.field_index(cty, fname)
            fd = s.tt.fields(cty)[i]
            g = s.op("getelementptr inbounds %s, ptr %s, i32 0, i32 %d" % (lty(cty), cur, i))
            boxed = s.fb(cty, None, fname)
            if k < len(path) - 1:
                cur = s.op("load ptr, ptr %s" % g) if boxed else g
                cty = fd.type
                continue
            ft = fd.type
            if boxed:
                old = s.op("load ptr, ptr %s" % g)
                s.ins("call void @pruv_dropnode__%s(ptr %s)" % (ft.name, old))
                nd = s.op("call ptr @pruv_box__%s(%s %s)" % (ft.name, lty(ft), v.a))
                s.ins("store ptr %s, ptr %s" % (nd, g))
            elif is_arr(ft):
                x = s.op("load %s, ptr %s" % (ARR, g))
                s.free_val(s.unpack(x, ft, False))      # the old buffer is freed with its cap
                s.ins("store %s %s, ptr %s" % (ARR, s.agg(v), g))
            elif s.is_ext(ft):
                s.ins("call void @pruv_drop__%s(ptr %s)" % (ft.name, g))
                s.ins("store %s %s, ptr %s" % (lty(ft), v.a, g))
            else:
                s.store_field(g, ft, v)

    def free_val(s, v):
        # The *_nn wrappers let the optimizer drop the call when the moved-out slot is null.
        if v.k == "coll":  # G6 A3: a fresh (non-null) collection temporary
            s.g6_drop(v)
        elif v.k == "agg":  # G9 A3: an extended temporary
            s.drop_agg(v)
        elif v.layout != "i64":
            sz, al = elem_size(v.layout), alignment(v.layout)
            s.ins("call void @pruv_free_v2_nn(ptr %s, i64 %s, i64 %d, i64 %d)" % (v.a, capof(v), sz, al))
        elif is_bytes(v):  # G2 A5
            s.ins("call void @pruv_free_v2_nn(ptr %s, i64 %s, i64 1, i64 1)" % (v.a, capof(v)))
        else:
            s.ins("call void @pruv_free_nn(ptr %s, i64 %s)" % (v.a, capof(v)))   # P3: the size is the cap, not the len

    def free_scope(s, sc):
        for sl in reversed(sc["arrays"]):
            if sl.a in s.ret_moved:
                s.drop_rest(sl, s.ret_moved[sl.a])
            elif sl.k == "coll":
                s.drop_slot(sl)
            elif sl.k == "agg" and is_owned_result(sl.ty):     # LG12 E2: an owned Result<T, String> local
                s.drop_oresult(sl.ty, sl.a)
            elif sl.k == "agg":     # G9 A3: an owned extended local
                s.ins("call void @pruv_drop__%s(ptr %s)" % (sl.ty.name, sl.a))
            else:
                s.free_val(s.load(sl))

    def drop_handle(s, h):
        """drop the String handle h unless it is null (the Ok side, or a String that was moved out)"""
        nn = s.op("icmp ne ptr %s, null" % h)
        dl, el = s.label("gdrop"), s.label("gdone")
        s.condbr(nn, dl, el)
        s.place(dl)
        s.g6_drop(V("coll", h, None, Type("string")))
        s.br(el)
        s.place(el)

    def drop_oresult(s, ty, p):
        """LG12 E2: release the owned value of type Result<T, String> stored at p: its String handle, when non-null"""
        h = s.op("load ptr, ptr %s" % g9_gep(s, lty(ty), p, 2))
        s.drop_handle(h)

    def mk_octor(s, e):
        """Ok / Err of Result<T, String>: Ok carries a value; Err MOVES its String in (a named String local becomes
        null, a fresh String is taken as is: both are owned by the result afterwards)"""
        ty = e.ty
        T = st(ty)
        if e.name == "Ok":
            x = s.ev(e.arg)
            acc = s.op("insertvalue %s zeroinitializer, i1 true, 0" % T)
            return V("agg", s.put(acc, ty, 1, x, ty.args[0], False), None, ty)
        a = e.arg
        if isinstance(a, Var) and s.lookup(a.name)[0] != "slot":
            raise PruvError("codegen", e.line, e.col, "unsupported_type",
                            "Err of the borrowed String %s needs the Rust backend" % a.name, "use the Rust backend",
                            file=getattr(e, "file", None))
        x = s.ev(a, True)
        if isinstance(a, Var):
            s.ins("store ptr null, ptr %s" % s.lookup(a.name)[1].a)
        return V("agg", s.op("insertvalue %s zeroinitializer, ptr %s, 2" % (T, x.a)), None, ty)

    def owned_match(s, st):
        """LG12 E2: `match r { Ok(v) => .., Err(e) => .. }` on a Result<T, String> CONSUMES r.  The tag, the payload and
        the String handle are read first; a local scrutinee then has its handle slot nulled (so its scope drop frees
        nothing), a temporary scrutinee (a call result, a constructor) is not dropped at all.  The Err binder becomes an
        owned String local of the arm scope: dropped at arm end unless it is moved (string_to_str take, Err(e), a take
        argument).  Other temporaries of the scrutinee are dropped right after it is evaluated."""
        sc = st.scrutinee
        ty = sc.ty
        X = ty.args[0]
        if isinstance(sc, Var) and s.lookup(sc.name)[0] != "slot":
            raise PruvError("codegen", st.line, st.col, "unsupported_type",
                            "a match on the borrowed Result<_, String> %s needs the Rust backend" % sc.name,
                            "use the Rust backend", file=getattr(st, "file", None))
        s.begin_temps()
        sv = s.ev(sc)
        for tv in reversed(s.temps.pop()):
            if tv is not sv:
                s.free_val(tv)
        T = lty(ty)
        tag = s.op("extractvalue %s %s, 0" % (T, sv.a))
        okv = s.get(sv, ty, 1, X, False)
        h = s.op("extractvalue %s %s, 2" % (T, sv.a))
        if isinstance(sc, Var):
            s.ins("store ptr null, ptr %s" % g9_gep(s, T, sv.p, 2))
        pos = [a for a in st.arms if a.ctor == "Ok"][0]
        neg = [a for a in st.arms if a.ctor == "Err"][0]
        pl, nl_, end = s.label("armT"), s.label("armF"), s.label("endmatch")
        s.condbr(tag, pl, nl_)
        reach = False
        for arm, lbl in ((pos, pl), (neg, nl_)):
            s.place(lbl)
            bind, own = None, None
            if arm.ctor == "Ok":
                if arm.binders:
                    bind = {arm.binders[0]: ("ssa", okv)}
            elif arm.binders:
                sl = s.new_slot(arm.binders[0], Type("string"))
                s.ins("store ptr %s, ptr %s" % (h, sl.a))
                bind, own = {arm.binders[0]: ("slot", sl)}, [sl]
            else:
                s.drop_handle(h)
            if s.run_block(arm.body, bind, own=own):
                s.br(end)
                reach = True
        if reach:
            s.place(end)
        else:
            s.dead = True

    def drop_rest(s, sl, moved):
        """P3: release an owned extended slot whose value (moved is None) or some unboxed payload fields of its
        current variant (moved = {storage index}) a return moved out: only the parts that were not moved"""
        if moved is None:
            return
        vn, idxs = moved
        for i, fd in enumerate(s.tt.fields(sl.ty, vn)):
            ft = fd.type
            k = payload_index(s.tt, sl.ty, vn, i)
            if k in idxs or not (is_arr(ft) or s.is_ext(ft)):
                continue
            gp = g9_gep(s, lty(sl.ty), sl.a, k)
            if s.fb(sl.ty, vn, fd.name):
                s.ins("call void @pruv_dropnode__%s(ptr %s)" % (ft.name, s.op("load ptr, ptr %s" % gp)))
            elif is_arr(ft):
                x = s.op("load %s, ptr %s" % (ARR, gp))
                s.free_val(s.unpack(x, ft, False))
            else:
                s.ins("call void @pruv_drop__%s(ptr %s)" % (ft.name, gp))

    def owned_slot(s, sl):
        return any(sl is x for sc in s.scopes for x in sc["arrays"])

    def ret_movable(s, e):
        """P3 (the LLVM form of the Rust backend's return_movable / consuming match): a return is the last use of every
        local, so an owned extended local, or an unboxed extended payload binder of a match on one, that the returned
        expression names exactly once (counting every name rooted at the same owner) moves out in an owning position
        instead of being cloned; the cleanup of that return then releases only what was not moved (drop_rest)."""
        if s.cost or e is None:
            return {}
        counts = {}

        def walk(x):
            if isinstance(x, (Var, Move)):
                counts[x.name] = counts.get(x.name, 0) + 1
            for k in codegen.expr_kids(x):
                walk(k)
        walk(e)
        # only a name in a result position (the returned expression itself, or a constructor / struct literal field
        # of one) moves: its value then lands in the returned value on every path, never in a temporary that a
        # nested short-circuit or call releases before the return ends
        res = set()

        def place(x):
            if isinstance(x, (Var, Move)):
                res.add(x.name)
            elif isinstance(x, (Ctor, StructLit, EnumCtor)):
                for k in codegen.expr_kids(x):
                    place(k)
            elif isinstance(x, Call) and x.name == "copy" and len(x.args) == 1 and isinstance(x.args[0], Var):
                res.add(x.args[0].name)     # copy(x) of an owned local array / array binder moves x instead
            elif isinstance(x, Call) and x.name == "truncate" and len(x.args) == 2 and isinstance(x.args[0], Var):
                res.add(x.args[0].name)     # truncate(x, m) of an owned local array shrinks x in place instead
        place(e)
        roots = {}
        for n in counts:
            try:
                kind, x = s.lookup(n)
            except Exception:
                continue
            if kind == "slot":
                if (x.k == "agg" and s.is_ext(x.ty) or x.k == "arr") and s.owned_slot(x):
                    roots.setdefault(x.a, []).append((n, x, True))
                elif x.k in ("agg", "coll", "arr"):
                    roots.setdefault(x.a, []).append((n, None, True))
            elif kind == "ssa" and x.mv is not None:
                roots.setdefault(x.mv[0].a, []).append((n, x, False))
        out = {}
        for _a, ns in roots.items():
            # the owner named itself moves only alone; distinct payload binders of one owner are distinct fields
            if any(whole for _n, _x, whole in ns) and len(ns) > 1:
                continue
            for n, x, _whole in ns:
                if counts[n] == 1 and x is not None and n in res:
                    out[n] = x
        return out

    def takeable(s, name):
        """P3 M3: `name` is an owned local String (not a parameter, not borrowed): string_to_str of it may take the buffer"""
        try:
            kind, x = s.lookup(name)
        except Exception:
            return False
        if kind != "slot" or not s.owned_slot(x):
            return False
        if not ((x.k == "coll" and x.ty.name == "string") or (x.k == "arr" and x.ty is not None and x.ty.name == "str")):
            return False    # P3 M5: an unboxed String local is an owned str array slot
        return not any(x is y for y in s.scopes[0]["arrays"])     # scope 0 holds the owned (take) parameters

    def ret_takable(s, e):
        """P3 M3: the String locals whose `string_to_str(x)` is in a result position of the returned expression (the
        expression itself, or a field of a constructor / struct literal / enum constructor in it, the same place walk
        as ret_movable) and that the expression names exactly once; the return is their last use, so the call takes the
        buffer and the return's cleanup skips the slot"""
        if s.cost or e is None:
            return set()
        counts = {}

        def walk(x):
            if isinstance(x, (Var, Move)):
                counts[x.name] = counts.get(x.name, 0) + 1
            for k in codegen.expr_kids(x):
                walk(k)
        walk(e)
        res = set()

        def place(x):
            if isinstance(x, (Ctor, StructLit, EnumCtor)):
                for k in codegen.expr_kids(x):
                    place(k)
            elif isinstance(x, Call) and x.name == "string_to_str" and len(x.args) == 1 and isinstance(x.args[0], Var):
                res.add(x.args[0].name)
        place(e)
        return set(n for n in res if counts.get(n) == 1 and s.takeable(n))

    def take_src(s, st):
        """P3 M3: a let / assignment `y = string_to_str(x)` whose whole right side is that call, at the last use of the
        owned local String x (dead_after), takes x's buffer; returns {x}, else an empty set"""
        e = st.value
        if s.cost or not (isinstance(e, Call) and e.name == "string_to_str" and len(e.args) == 1):
            return set()
        x = e.args[0]
        if not isinstance(x, Var) or x.name == st.name or not s.dead_after(x.name) or not s.takeable(x.name):
            return set()
        return {x.name}

    def unbox_uses(s, name, e, contract=False):
        """P3 M5: every occurrence of `name` in the expression e is len(name) or string_to_str(name) (code only)"""
        if isinstance(e, (Var, Move)):
            return e.name != name
        if isinstance(e, Quant) and e.var == name:
            return False
        if isinstance(e, Call):
            if e.name == name:
                return False
            if (e.name in ("len", "string_to_str") and len(e.args) == 1 and isinstance(e.args[0], Var)
                    and e.args[0].name == name):
                return not contract
        return all(s.unbox_uses(name, k, contract) for k in codegen.expr_kids(e))

    def unbox_block(s, name, stmts):
        """P3 M5: no statement of the block (nested blocks included) binds, assigns or uses `name` other than through
        len(name) / string_to_str(name); a contract expression (assert, loop invariant / decreases) never names it"""
        for st in stmts:
            if isinstance(st, Let):
                if st.name == name or not s.unbox_uses(name, st.value):
                    return False
            elif isinstance(st, (Assign, FieldAssign)):
                if st.name == name or not s.unbox_uses(name, st.value):
                    return False
            elif isinstance(st, IndexAssign):
                if st.name == name or not s.unbox_uses(name, st.index) or not s.unbox_uses(name, st.value):
                    return False
            elif isinstance(st, If):
                if not (s.unbox_uses(name, st.cond) and s.unbox_block(name, st.then) and s.unbox_block(name, st.els)):
                    return False
            elif isinstance(st, While):
                if not (s.unbox_uses(name, st.cond) and all(s.unbox_uses(name, i, True) for i in st.invariants)
                        and s.unbox_uses(name, st.decreases, True) and s.unbox_block(name, st.body)):
                    return False
            elif isinstance(st, Return):
                if st.value is not None and not s.unbox_uses(name, st.value):
                    return False
            elif isinstance(st, Assert):
                if not s.unbox_uses(name, st.cond, True):
                    return False
            elif isinstance(st, ExprStmt):
                if not s.unbox_uses(name, st.expr):
                    return False
            elif isinstance(st, Par):
                if not all(s.unbox_uses(name, c) for c in st.calls):
                    return False
            elif isinstance(st, Match):
                if not s.unbox_uses(name, st.scrutinee):
                    return False
                for a in st.arms:
                    if name in a.binders or not s.unbox_block(name, a.body):
                        return False
            else:
                return False
        return True

    def tail_fill(s, st, nxt):
        """P3: `let mut x: [i64 | u8] = alloc(n, 0); let mut i: i64 = 0; while i < k { x[i] = e; i = i + 1; }` in
        verified mode (not cost mode), with k a name other than x and i and e a call-free expression not naming x:
        the loop overwrites x[0..min(k, n)) before any read, so the alloc zeroes only the rest.  Returns the k node or None"""
        if not s.verified or s.cost or not st.mutable or st.type is None:
            return None
        if st.type.name not in ("array", "array_u8"):
            return None
        e = st.value
        if not (isinstance(e, Call) and e.name == "alloc" and e.name not in s.funcs and len(e.args) == 2
                and isinstance(e.args[1], IntLit) and e.args[1].value == 0):
            return None
        # P3: statements x[0] = e0; x[1] = e1; ... (constant indices in order, each e call-free and not naming x)
        # overwrite x[0..j) before any read; the run's length is the bound
        j = 0
        while j < len(nxt) and isinstance(nxt[j], IndexAssign) and nxt[j].name == st.name \
                and isinstance(nxt[j].index, IntLit) and nxt[j].index.value == j and s.tail_pure(nxt[j].value, st.name):
            j += 1
        if j > 0:
            return dataclasses.replace(e.args[1], value=j)
        if len(nxt) < 2:
            return None
        li, w = nxt[:2]
        if not (isinstance(li, Let) and li.type is not None and li.type.name == "i64" and isinstance(li.value, IntLit)
                and li.value.value == 0 and li.name != st.name):
            return None
        x, i = st.name, li.name
        c = w.cond if isinstance(w, While) else None
        if not (isinstance(c, Binary) and c.op == "<" and isinstance(c.left, Var) and c.left.name == i
                and isinstance(c.right, Var) and c.right.name not in (x, i, "result")):
            return None
        if len(w.body) != 2:
            return None
        sa, inc = w.body
        if not (isinstance(sa, IndexAssign) and sa.name == x and isinstance(sa.index, Var) and sa.index.name == i
                and s.tail_pure(sa.value, x)):
            return None
        if not (isinstance(inc, Assign) and inc.name == i and isinstance(inc.value, Binary) and inc.value.op == "+"
                and isinstance(inc.value.left, Var) and inc.value.left.name == i
                and isinstance(inc.value.right, IntLit) and inc.value.right.value == 1):
            return None
        return c.right

    def tail_pure(s, e, x):
        """P3 (tail_fill): e has no call and does not name x"""
        if isinstance(e, (IntLit, BoolLit)):
            return True
        if isinstance(e, Var):
            return e.name != x
        if isinstance(e, Unary):
            return s.tail_pure(e.operand, x)
        if isinstance(e, Binary):
            return s.tail_pure(e.left, x) and s.tail_pure(e.right, x)
        if isinstance(e, Index):
            return s.tail_pure(e.array, x) and s.tail_pure(e.index, x)
        return False

    def unbox_let(s, st, rest):
        """P3 M5: `let x: String = f(args)` with a trusted foreign f returning String (not a catalog op, not cost mode)
        whose name is, in the rest of the block, only measured (len) or converted (string_to_str): x is kept as an owned
        str array filled by the adapter pruv_g6_v1_<f>__str, with no String box"""
        e = st.value
        if s.cost or st.type is None or st.type.name != "string" or not isinstance(e, Call):
            return False
        f = s.funcs.get(e.name)
        if f is None or f.foreign is None or e.name in CATALOG or not is_collection(f.ret) or f.ret.name != "string":
            return False
        return s.unbox_block(st.name, rest)

    def hold(s, v):
        """move a temporary into a fresh owner slot (returned); the caller registers it in a scope"""
        if v.k == "agg":
            sl = s.new_slot("hold", v.ty)
        elif v.k == "coll":
            sl = s.new_slot("hold", v.ty)
        else:
            sl = s.new_slot("hold", ARRAY)
            sl.ty = v.ty
        s.store(sl, v)
        return sl

    # ---------- G6 A3: shim calls, owner slots ----------
    def g6_decl(s, sym, ret, argtys):
        s.m.g6_decls[sym] = "declare %s @%s(%s) nounwind" % (ret, sym, ", ".join(argtys))
        s.bridge = True

    def g6_call(s, sym, ret, args):
        """call a bridge symbol (sym is the unprefixed foreign_shim name; the program prefix is added here);
        args: [(llvm type, value)]; returns the SSA result (None for void)"""
        sym = s.m.sym_pre + sym
        s.g6_decl(sym, ret, [t for t, _ in args])
        a = ", ".join("%s %s" % (t, v) for t, v in args)
        if ret == "void":
            s.ins("call void @%s(%s)" % (sym, a))
            return None
        return s.op("call %s @%s(%s)" % (ret, sym, a))

    def g6_drop(s, v):
        s.g6_call(foreign_shim.drop_sym(v.ty), "void", [("ptr", v.a)])

    def drop_slot(s, sl):
        """drop the owner slot's collection if it is non-null (moved-out slots hold null)"""
        p = s.op("load ptr, ptr %s" % sl.a)
        nn = s.op("icmp ne ptr %s, null" % p)
        dl, el = s.label("gdrop"), s.label("gdone")
        s.condbr(nn, dl, el)
        s.place(dl)
        s.g6_drop(V("coll", p, None, sl.ty))
        s.br(el)
        s.place(el)

    def pop_scope(s):
        sc = s.scopes.pop()
        if not s.dead:
            s.free_scope(sc)

    def begin_temps(s):
        s.temps.append([])

    def end_temps(s):
        ts = s.temps.pop()
        if not s.dead:
            for v in reversed(ts):
                s.free_val(v)

    # ---------- user aggregates (G1) ----------
    def pack(s, v, ty, c):
        """SSA value of storage type st(ty, c) for the value v"""
        if ty.name in ("option", "result") and not gopt(ty):
            ot, pt = (COPT, "i128") if c else (OPT, "i64")
            a = s.op("insertvalue %s undef, i1 %s, 0" % (ot, v.a))
            return s.op("insertvalue %s %s, %s %s, 1" % (ot, a, pt, v.b))
        if is_arr(ty):  # G9 A3: a buffer field is stored as { ptr, i64, i64 } (data, len, cap)
            return s.agg(v)
        return v.a

    def store_field(s, ptr, ty, v):
        s.ins("store %s %s, ptr %s" % (st(ty), s.pack(v, ty, False), ptr))

    def unpack(s, x, ty, c):
        n = ty.name
        if is_arr(ty):  # G9 A3
            return V("arr", s.op("extractvalue %s %s, 0" % (ARR, x)), s.op("extractvalue %s %s, 1" % (ARR, x)),
                     elem_ty(ty), c=s.op("extractvalue %s %s, 2" % (ARR, x)))
        if n == "i64":
            return V("i128" if c else "i64", x)
        if n == "bool":
            return V("i1", x)
        if n in ("option", "result") and not gopt(ty):
            ot = COPT if c else OPT
            t = s.op("extractvalue %s %s, 0" % (ot, x))
            p = s.op("extractvalue %s %s, 1" % (ot, x))
            return V("opt", t, p)
        return V("agg", x, None, ty)

    def put(s, base, ty, idx, v, fty, c):
        """insertvalue of field value v (type fty) at storage index idx of aggregate `base` (type ty)"""
        x = s.pack(v, fty, c)
        return s.op("insertvalue %s %s, %s %s, %d" % (st(ty, c), base, st(fty, c), x, idx))

    def get(s, base, ty, idx, fty, c):
        x = s.op("extractvalue %s %s, %d" % (st(ty, c), base.a, idx))
        return s.unpack(x, fty, c)

    def tag_of(s, v, c):
        return s.op("extractvalue %s %s, 0" % (st(v.ty, c), v.a))

    def mk_struct(s, e, evf, c):
        ty = Type(e.name)
        vals = {}
        for fi in e.fields:                 # source order evaluation
            vals[fi.name] = evf(fi.value)
        acc = "zeroinitializer"
        for i, fd in enumerate(s.tt.fields(ty)):
            acc = s.putf(acc, ty, i, vals[fd.name], fd, None, c)    # G9: a boxed field moves into a new node
        return V("agg", acc, None, ty)

    def mk_enum(s, e, evf, c):
        ty = Type(e.enum_name)
        vals = [evf(x) for x in e.args]     # left to right
        tag = s.tt.variant_index(ty, e.variant)
        acc = s.op("insertvalue %s zeroinitializer, i64 %d, 0" % (st(ty, c), tag))
        for i, fd in enumerate(s.tt.fields(ty, e.variant)):
            acc = s.putf(acc, ty, payload_index(s.tt, ty, e.variant, i), vals[i], fd, e.variant, c)
        return V("agg", acc, None, ty)

    def mk_gctor(s, e, evf, c):
        """Some/None/Ok/Err of a new Option<X>/Result<X> application (e.ty): zeroinitializer-based, so the unused
        payload storage is always zero"""
        ty = e.ty
        if e.name == "None":
            return V("agg", "zeroinitializer", None, ty)
        x = evf(e.arg)
        if e.name in ("Some", "Ok"):
            acc = s.op("insertvalue %s zeroinitializer, i1 true, 0" % st(ty, c))
            return V("agg", s.put(acc, ty, 1, x, ty.args[0], c), None, ty)
        # Err: tag false (zero), error payload in slot 2
        return V("agg", s.put("zeroinitializer", ty, 2, x, result_error(ty), c), None, ty)

    def gmatch(s, st_, sv):
        """match on a new Option<X>/Result<X>: branch on the tag, bind the active payload only inside its arm"""
        ty = sv.ty
        X = ty.args[0]
        pos = [a for a in st_.arms if a.ctor in ("Some", "Ok")][0]
        neg = [a for a in st_.arms if a.ctor in ("None", "Err")][0]
        tag = s.op("extractvalue %s %s, 0" % (st(ty), sv.a))
        pl, nl_, end = s.label("armT"), s.label("armF"), s.label("endmatch")
        s.condbr(tag, pl, nl_)
        reach = False
        for arm, lbl in ((pos, pl), (neg, nl_)):
            s.place(lbl)
            bind = None
            if arm.binders:
                if arm.ctor in ("Some", "Ok"):
                    bind = {arm.binders[0]: ("ssa", s.get(sv, ty, 1, X, False))}
                else:
                    bind = {arm.binders[0]: ("ssa", s.get(sv, ty, 2, result_error(ty), False))}
            if s.run_block(arm.body, bind):
                s.br(end)
                reach = True
        if reach:
            s.place(end)
        else:
            s.dead = True

    def set_path(s, base, ty, path, v):
        """aggregate `base` (code form) with the nested struct field `path` replaced by v"""
        fds = s.tt.fields(ty)
        i = s.tt.field_index(ty, path[0])
        fty = fds[i].type
        if len(path) == 1:
            nv = v
        else:
            sub = s.get(base, ty, i, fty, False)
            nv = s.set_path(sub, fty, path[1:], v)
        return V("agg", s.put(base.a, ty, i, nv, fty, False), None, ty)

    def to_c(s, x, ty):
        """code value -> contract value (i64 leaves sign-extended; enums convert the active variant only)"""
        n = ty.name
        if n == "i64":
            return V("i128", s.op("sext i64 %s to i128" % x.a))
        if n == "bool":
            return x
        if gopt(ty):  # G3: widen the payload (and the Result error); the unused payload stays zero
            X = ty.args[0]
            t = s.op("extractvalue %s %s, 0" % (st(ty), x.a))
            acc = s.op("insertvalue %s zeroinitializer, i1 %s, 0" % (st(ty, True), t))
            acc = s.put(acc, ty, 1, s.to_c(s.get(x, ty, 1, X, False), X), X, True)
            if n == "result":
                E = result_error(ty)
                acc = s.put(acc, ty, 2, s.to_c(s.get(x, ty, 2, E, False), E), E, True)
            return V("agg", acc, None, ty)
        if n in ("option", "result"):
            return V("opt", x.a, s.op("sext i64 %s to i128" % x.b))
        d = s.tt.decl(ty)
        if isinstance(d, StructDecl):
            acc = "zeroinitializer"
            for i, f in enumerate(d.fields):
                fx = s.get(x, ty, i, f.type, False)
                acc = s.put(acc, ty, i, s.to_c(fx, f.type), f.type, True)
            return V("agg", acc, None, ty)
        tag = s.tag_of(x, False)
        base = s.op("insertvalue %s zeroinitializer, i64 %s, 0" % (st(ty, True), tag))
        vs = [v for v in d.variants if v.fields]
        if not vs:
            return V("agg", base, None, ty)
        lcur, join = s.cur, s.label("cvend")
        lbls = [s.label("cvv") for _ in vs]
        s.term("switch i64 %s, label %%%s [ %s ]" % (tag, join, " ".join(
            "i64 %d, label %%%s" % (s.tt.variant_index(ty, v.name), l) for v, l in zip(vs, lbls))))
        entries = [(base, lcur)]
        for v, l in zip(vs, lbls):
            s.place(l)
            acc = base
            for i, f in enumerate(v.fields):
                pi = payload_index(s.tt, ty, v.name, i)
                fx = s.get(x, ty, pi, f.type, False)
                acc = s.put(acc, ty, pi, s.to_c(fx, f.type), f.type, True)
            entries.append((acc, s.cur))
            s.br(join)
        s.place(join)
        r = s.op("phi %s %s" % (st(ty, True), ", ".join("[ %s, %%%s ]" % e for e in entries)))
        return V("agg", r, None, ty)

    def ceq(s, l, r, ty):
        """structural equality of two contract values of type ty -> i1 (SSA name or literal)"""
        n = ty.name
        if n == "i64":
            return s.op("icmp eq i128 %s, %s" % (l.a, r.a))
        if n == "bool":
            return s.op("icmp eq i1 %s, %s" % (l.a, r.a))
        if gopt(ty):  # G3: only the active payload is compared
            X = ty.args[0]
            T = st(ty, True)
            tl = s.op("extractvalue %s %s, 0" % (T, l.a))
            tr = s.op("extractvalue %s %s, 0" % (T, r.a))
            te = s.op("icmp eq i1 %s, %s" % (tl, tr))
            pe = s.ceq(s.get(l, ty, 1, X, True), s.get(r, ty, 1, X, True), X)
            nt = s.op("xor i1 %s, true" % tl)
            if n == "option":
                return s.op("and i1 %s, %s" % (te, s.op("or i1 %s, %s" % (nt, pe))))
            E = result_error(ty)
            ee = s.ceq(s.get(l, ty, 2, E, True), s.get(r, ty, 2, E, True), E)
            act = s.op("or i1 %s, %s" % (s.op("and i1 %s, %s" % (tl, pe)), s.op("and i1 %s, %s" % (nt, ee))))
            return s.op("and i1 %s, %s" % (te, act))
        if n in ("option", "result"):
            te = s.op("icmp eq i1 %s, %s" % (l.a, r.a))
            pe = s.op("icmp eq i128 %s, %s" % (l.b, r.b))
            if n == "option":           # a None payload is not compared
                nt = s.op("xor i1 %s, true" % l.a)
                pe = s.op("or i1 %s, %s" % (nt, pe))
            return s.op("and i1 %s, %s" % (te, pe))

        def fields_eq(fds, pidx):
            acc = "true"
            for i, f in enumerate(fds):
                j = pidx(i)
                e = s.ceq(s.get(l, ty, j, f.type, True), s.get(r, ty, j, f.type, True), f.type)
                acc = e if acc == "true" else s.op("and i1 %s, %s" % (acc, e))
            return acc
        d = s.tt.decl(ty)
        if isinstance(d, StructDecl):
            return fields_eq(d.fields, lambda i: i)
        tl, tr = s.tag_of(l, True), s.tag_of(r, True)
        teq = s.op("icmp eq i64 %s, %s" % (tl, tr))
        vs = [v for v in d.variants if v.fields]
        if not vs:
            return teq
        lcur = s.cur
        same, dflt, end = s.label("ceqs"), s.label("ceqd"), s.label("ceqe")
        lbls = [s.label("ceqv") for _ in vs]
        s.condbr(teq, same, end)
        entries = [("false", lcur)]
        s.place(same)
        s.term("switch i64 %s, label %%%s [ %s ]" % (tl, dflt, " ".join(
            "i64 %d, label %%%s" % (s.tt.variant_index(ty, v.name), lb) for v, lb in zip(vs, lbls))))
        s.place(dflt)           # same tag, variant without fields
        entries.append(("true", dflt))
        s.br(end)
        for v, lb in zip(vs, lbls):
            s.place(lb)
            e = fields_eq(v.fields, lambda i, v=v: payload_index(s.tt, ty, v.name, i))
            entries.append((e, s.cur))
            s.br(end)
        s.place(end)
        return s.op("phi i1 %s" % ", ".join("[ %s, %%%s ]" % e for e in entries))

    def agg(s, v):
        if v.k == "agg":
            return v.a
        if v.k == "arr":
            a = s.op("insertvalue %s undef, ptr %s, 0" % (ARR, v.a))
            a = s.op("insertvalue %s %s, i64 %s, 1" % (ARR, a, v.b))
            return s.op("insertvalue %s %s, i64 %s, 2" % (ARR, a, capof(v)))
        a = s.op("insertvalue %s undef, i1 %s, 0" % (OPT, v.a))
        return s.op("insertvalue %s %s, i64 %s, 1" % (OPT, a, v.b))

    # ---------- function frame ----------
    def bind_params(s):
        s.place("start")
        names = {}
        owned = []
        for p in s.f.params:
            n = mangle(p.name)
            if getattr(p, "resizable", False):
                names[p.name] = ("slot", s.array_slot("%%p.%s" % n, p.type))
            elif p.mode == "inout" and p.type in (I64, BOOL):
                names[p.name] = ("slot", Slot(vkind(p.type), "%%p.%s" % n))
            elif is_arr(p.type):
                v = V("arr", "%%p.%s.a" % n, "%%p.%s.n" % n, elem_ty(p.type),
                      layout=s.m.storage.layout_of(p), rng=s.m.storage.range_of(p),
                      c="%%p.%s.c" % n if p.take else None)
                if p.take:
                    sl = s.new_slot(p.name, p.type, packed=p.name in s.resize_args)
                    s.store(sl, v)
                    names[p.name] = ("slot", sl)
                    owned.append(sl)
                else:
                    names[p.name] = ("ssa", v)
            elif p.type == BOOL:
                names[p.name] = ("ssa", V("i1", "%%p.%s" % n))
            elif p.type == I64:
                names[p.name] = ("ssa", V("i64", "%%p.%s" % n))
            elif is_collection(p.type):  # G6 A3
                s.bridge = True
                if p.take:   # consuming parameter: an owner slot (the callee drops it unless it is moved out)
                    sl = s.new_slot(p.name, p.type)
                    s.ins("store ptr %%p.%s.h, ptr %s" % (n, sl.a))
                    names[p.name] = ("slot", sl)
                    owned.append(sl)
                else:        # borrowed: never dropped here
                    names[p.name] = ("ssa", V("coll", "%%p.%s.h" % n, None, p.type))
            elif p.mode == "inout" and is_agg(p.type):
                names[p.name] = ("slot", Slot("agg", "%%p.%s" % n, None, p.type))
            elif s.is_ext(p.type) and p.take:   # LG9: the callee owns the storage; dropped at exit unless moved out
                sl = Slot("agg", "%%p.%s" % n, None, p.type)
                names[p.name] = ("slot", sl)
                owned.append(sl)
            elif s.is_ext(p.type):   # G9 A3: a read view of caller-owned storage; never dropped here
                pv = s.op("load %s, ptr %%p.%s" % (lty(p.type), n))
                names[p.name] = ("ssa", V("agg", pv, None, p.type, "%%p.%s" % n))
            elif is_agg(p.type):
                names[p.name] = ("ssa", V("agg", "%%p.%s" % n, None, p.type))
            else:
                t = s.op("extractvalue %s %%p.%s, 0" % (OPT, n))
                v = s.op("extractvalue %s %%p.%s, 1" % (OPT, n))
                names[p.name] = ("ssa", V("opt", t, v))
        s.scopes.append({"names": names, "arrays": owned})

    def emit_ret(s, v):
        if s.cost:
            s.cost_check()
        rt = s.f.ret
        if s.is_ext(rt):    # G9 A3: explicit output storage; v is an owned value
            s.ins("store %s %s, ptr %%out" % (lty(rt), v.a))
            s.term("ret void")
        elif rt == UNIT:
            s.term("ret void")
        elif rt in (I64, BOOL) or is_collection(rt):
            s.term("ret %s %s" % (lty(rt), v.a))
        else:
            g = s.agg(v)
            s.term("ret %s %s" % (lty(rt), g))

    # ---------- statements ----------
    def run_block(s, stmts, bind=None, loop=False, own=None):
        """lower a block in its own scope; returns True if control can fall through.  own: owner slots (LG12 E2: the
        String binder of an Err arm) released when the scope ends"""
        s.push_scope()
        if bind:
            s.scopes[-1]["names"].update(bind)
        if own:
            s.scopes[-1]["arrays"].extend(own)
        later = [set() for _ in range(len(stmts) + 1)]
        for i in range(len(stmts) - 1, -1, -1):
            later[i] = set(later[i + 1])
            codegen._stmt_names([stmts[i]], later[i])
        frame = [set(bind or ()), set(), loop, False]
        s.frames.append(frame)
        for i, st in enumerate(stmts):
            frame[1] = later[i + 1]
            frame[3] = i + 1 < len(stmts) and isinstance(stmts[i + 1], Return)
            s.unbox_st = st if isinstance(st, Let) and s.unbox_let(st, stmts[i + 1:]) else None
            s.tail_k = s.tail_fill(st, stmts[i + 1:]) if isinstance(st, Let) else None
            s.tail_st = st if s.tail_k is not None else None
            s.stmt(st)
            if isinstance(st, Let):
                frame[0].add(st.name)
            if s.dead:
                break
        s.frames.pop()
        s.pop_scope()
        return not s.dead

    def dead_after(s, name):
        """P3 (the Rust backend's dead_after): the statement being lowered is the last use of local `name`: no later
        statement of any enclosing block up to the one declaring it (or binding it, for a match arm) names it, and no
        loop body lies between; a parameter is never dead here"""
        for decls, later, loop, _ in reversed(s.frames):
            if name in later:
                return False
            if name in decls:
                return True
            if loop:
                return False
        return False

    def return_next(s, name):
        """LG34 (G-LG33-2, the Rust backend's return_next): the statement after the one being lowered is a return of the
        same block that does not name local `name`, so `name` is dead even inside a loop body (control leaves the
        function before any later iteration); an owned (take) parameter in scope 0 is never dead here"""
        if not s.frames or not s.frames[-1][3] or name in s.frames[-1][1]:
            return False
        kind, v = s.lookup(name)
        return kind == "slot" and not any(v is y for y in s.scopes[0]["arrays"])

    def move_src(s, st):
        """P3 (the Rust backend's last_copy): a let / assignment `y = copy(x)` at the last use of x moves x when x is an
        owned local array, or an array payload binder of a match on an owned extended local that is dead after the
        match; returns {x} (the copy lowering then takes the value and empties its owner), else an empty set"""
        e = st.value
        if (not s.cost and isinstance(e, Call) and len(e.args) == (2 if e.name == "truncate" else 1)
                and e.name in ("truncate", "str_from_utf8") and isinstance(e.args[0], Var)):
            # LG33 (G-LG32-5, G-DB54-2): truncate(x, m) / str_from_utf8(x) of an owned local array at its last use
            # hands x's block over (the Rust backend's last_copy); a length argument naming x keeps it borrowed
            x = e.args[0]
            names = set()
            if e.name == "truncate":
                codegen._expr_names(e.args[1], names)
            if x.name == st.name or x.name in names or not (s.dead_after(x.name) or s.return_next(x.name)):
                return set()
            kind, v = s.lookup(x.name)
            return {x.name} if kind == "slot" and v.k == "arr" and s.owned_slot(v) else set()
        if s.cost or not (isinstance(e, Call) and e.name == "copy" and len(e.args) == 1):
            return set()
        x = e.args[0]
        if (isinstance(x, FieldAccess) and isinstance(x.base, Var) and x.base.name != st.name
                and s.dead_after(x.base.name)):
            # copy(v.f) of an unboxed array field of an owned extended struct local v at its last use moves the
            # field out of v (the Rust backend's last_copy); v's cleanup then frees the emptied field as null
            kind, v = s.lookup(x.base.name)
            if (kind == "slot" and v.k == "agg" and s.is_ext(v.ty) and s.owned_slot(v)
                    and isinstance(s.tt.decl(v.ty), StructDecl)):
                fd = s.tt.fields(v.ty)[s.tt.field_index(v.ty, x.field)]
                if is_arr(fd.type) and not s.fb(v.ty, None, fd.name):
                    return {x.base.name}
            return set()
        if not isinstance(x, Var) or x.name == st.name or not s.dead_after(x.name):
            return set()
        kind, v = s.lookup(x.name)
        if kind == "slot" and v.k == "arr" and s.owned_slot(v):
            return {x.name}
        if kind == "ssa" and v.k == "arr" and v.mv is not None and v.mv[3]:
            return {x.name}
        return set()

    def copy_move(s, e):
        """P3: the value of copy(x) when x moves instead (a return candidate or a move_src name), else None"""
        if isinstance(e, FieldAccess) and isinstance(e.base, Var) and e.base.name in s.moving:
            sl = s.lookup(e.base.name)[1]    # move_src: an owned struct slot; the field moves out and is emptied
            i = s.tt.field_index(sl.ty, e.field)
            gp = g9_gep(s, lty(sl.ty), sl.a, i)
            x = s.op("load %s, ptr %s" % (ARR, gp))
            s.ins("store %s zeroinitializer, ptr %s" % (ARR, gp))
            return s.unpack(x, s.tt.fields(sl.ty)[i].type, False)
        if not isinstance(e, Var) or (e.name not in s.ret_cand and e.name not in s.moving):
            return None
        kind, x = s.lookup(e.name)
        if kind == "slot":
            v = s.load(x)
            if e.name in s.ret_cand:
                s.ret_moved[x.a] = None     # the return's cleanup skips the slot
            else:
                s.empty_array_slot(x)
            return v
        sl, vn, k, _dead = x.mv
        if e.name in s.ret_cand:
            prev = s.ret_moved.get(sl.a)
            s.ret_moved[sl.a] = (vn, (prev[1] if prev else set()) | {k})
        else:
            s.ins("store %s zeroinitializer, ptr %s" % (ARR, g9_gep(s, lty(sl.ty), sl.a, k)))
        return x

    def empty_array_slot(s, x):
        s.ins("store ptr null, ptr %s" % x.a)
        s.ins("store i64 0, ptr %s" % x.b)
        s.ins("store i64 0, ptr %s" % x.c)

    def body(s):
        s.bind_params()
        if s.cost:
            args = ["%s %s" % (t, n) for t, n in ir_params(s.f, s.tt) if n != "%out"]
            s.cost_bound = s.op("call i128 @%s__pruv_cost(%s)" % (mangle(s.f.name), ", ".join(args)))
            s.allocas.append("%cost.spent = alloca i128")
            s.ins("store i128 0, ptr %cost.spent")
        falls = s.run_block(s.f.body)
        if falls:
            if s.f.ret == UNIT:
                s.free_scope(s.scopes[0])   # G6 A3: owned (take) parameters; empty for every other program
                s.emit_ret(V("unit"))
            else:
                s.term("unreachable")

    def stack_let(s, st):
        """P3-S1: `let x: [E] = alloc(C, v)` on the stack: an entry-block alloca of C elements,
        filled every time the declaration runs: @llvm.memset for byte elements (size 1) and literal-zero fills,
        pruv_fill_v2 otherwise, and nothing for length 0; the slot is not owned, so nothing is ever freed."""
        n = s.m.stack.length(st)
        s.begin_temps()
        v = s.ev(st.value.args[1])      # n is a literal (no code); the fill value is evaluated here, once
        if st.type.name == "array_bool":
            val = v.a                   # i1; widened to the fill byte below (no code at all when n == 0)
            layout, size, ety, rng = "i64", 1, "i8", None
        elif st.type.name == "array_u8":
            if not s.verified:
                ok = s.op("icmp ule i64 %s, 255" % v.a)
                s.guard(ok, s.panic_elem(v.a), False)
            val = v.a
            layout, size, ety, rng = "i64", 1, "i8", None
        else:
            layout = s.m.storage.layout_of(st)
            rng = s.m.storage.range_of(st)
            size = elem_size(layout)
            val, ety = v.a, "i%d" % (size * 8)
        sl = s.new_slot(st.name, st.type)
        buf = sl.a[:-2] + ".stk"
        s.allocas.append("%s = alloca [%d x %s], align %d" % (buf, n, ety, size))
        if n == 0:
            pass                        # nothing to fill
        elif size == 1:
            if st.type.name == "array_bool":
                byte = s.op("zext i1 %s to i8" % val)
            else:
                byte = s.op("trunc i64 %s to i8" % val)
            s.ins("call void @llvm.memset.p0.i64(ptr align 1 %s, i8 %s, i64 %d, i1 false)" % (buf, byte, n))
        elif val == "0":
            s.ins("call void @llvm.memset.p0.i64(ptr align %d %s, i8 0, i64 %d, i1 false)" % (size, buf, n * size))
        else:
            s.m.fill = True
            s.ins("call void @pruv_fill_v2(ptr %s, i64 %d, i64 %s, i64 %d)" % (buf, n, val, size))
        s.store(sl, V("arr", buf, str(n), elem_ty(st.type), layout=layout, rng=rng))
        s.scopes[-1]["names"][st.name] = ("slot", sl)
        s.end_temps()

    def stmt(s, st):
        if isinstance(st, Let) and s.m.stack is not None and s.m.stack.contains(st) and st.name not in s.resize_args:
            s.stack_let(st)
        elif isinstance(st, Let) and s.m.stack is not None and s.m.stack.lit(st) and not s.cost:
            # P3: a borrowed literal local binds the read-only global directly: no copy, never freed
            s.begin_temps()
            v = s.ev(st.value, False)
            sl = s.new_slot(st.name, st.type)
            s.store(sl, v)
            s.scopes[-1]["names"][st.name] = ("slot", sl)
            s.end_temps()
        elif isinstance(st, Let) and st is s.unbox_st:
            # P3 M5: an unboxed String local: the adapter fills an owned str array (null, 0 when empty), no box
            s.begin_temps()
            f = s.funcs[st.value.name]
            args = []
            for x in st.value.args:
                args += s.g6_arg(s.ev(x))
            d, l = s.g6_out("ptr"), s.g6_out("i64")
            s.m.g6_unbox.add(f.name)
            s.m.g6_v2 = True
            s.g6_call(foreign_shim.unbox_adapter_sym(f.name), "void", args + [("ptr", d), ("ptr", l)])
            dp = s.op("load ptr, ptr %s" % d)
            ln = s.op("load i64, ptr %s" % l)
            sl = s.new_slot(st.name, STR)
            s.store(sl, V("arr", dp, ln, STR))
            sc = s.scopes[-1]
            sc["names"][st.name] = ("slot", sl)
            sc["arrays"].append(sl)
            s.end_temps()
        elif isinstance(st, Let):
            s.begin_temps()
            s.moving = s.move_src(st)
            s.take_let = s.take_src(st)
            v = s.ev(st.value, True)
            s.moving = set()
            s.take_let = set()
            sl = s.new_slot(st.name, st.type, packed=st.name in s.resize_args)
            s.store(sl, v)
            sc = s.scopes[-1]
            sc["names"][st.name] = ("slot", sl)
            if sl.k == "arr" or sl.k == "coll" or (sl.k == "agg" and s.is_ext(sl.ty)):
                sc["arrays"].append(sl)     # an owner released when the scope ends
            s.end_temps()
        elif isinstance(st, Assign):
            s.begin_temps()
            sl = s.lookup(st.name)[1]
            # LG32 (G-DB52-4): x = resize(x, n, v) grows the slot's block in place (realloc), so the old and new blocks are never live together
            if s.regrow_assign(st, sl):
                s.end_temps()
                return
            s.moving = s.move_src(st)
            s.take_let = s.take_src(st)
            v = s.ev(st.value, True)        # the right side is complete and owned before the old value is destroyed
            s.moving = set()
            s.take_let = set()
            if sl.k == "arr":
                s.free_val(s.load(sl))
            elif sl.k == "coll":   # G6 A3: drop the old value (null when moved out) after the right side
                s.drop_slot(sl)
            elif sl.k == "agg" and is_owned_result(sl.ty):   # LG12 E2: the old Result releases its String
                s.drop_oresult(sl.ty, sl.a)
            elif sl.k == "agg" and s.is_ext(sl.ty):   # G9 A3
                s.ins("call void @pruv_drop__%s(ptr %s)" % (sl.ty.name, sl.a))
            s.store(sl, v)
            s.end_temps()
        elif isinstance(st, FieldAssign):
            s.begin_temps()
            kind, sl = s.lookup(st.name)
            if kind != "slot":
                raise Exception("llvmgen: field assignment to non-local %s" % st.name)
            if s.regrow_field(st, sl):         # LG34 (G-LG32-3): x.f = resize(x.f, n, v) grows the field in place
                s.end_temps()
                return
            if s.is_ext(sl.ty):             # G9 A3: owned right side first, then an in-place store along the path
                v = s.ev(st.value, True)
                s.assign_path(sl, st.fields, v)
                s.end_temps()
                return
            v = s.ev(st.value)
            if s.tt.is_struct(sl.ty):
                gp, fty = s.field_ptr_ty(sl, st.fields)
                s.store_field(gp, fty, v)
            else:
                root = s.load(sl)
                s.store(sl, s.set_path(root, sl.ty, st.fields, v))
            s.end_temps()
        elif isinstance(st, IndexAssign):
            s.begin_temps()
            kind, sl = s.lookup(st.name)
            i = s.ev(st.index)      # index first, then value, in every mode
            v = s.ev(st.value)
            if st.fields:
                # LG4: `p.a.b[i] = e`: address of the buffer field along the path from the root slot (a local slot or
                # the pointer of an inout struct parameter), then the data pointer and length are read from it
                if kind != "slot":
                    raise Exception("llvmgen: element store through non-local %s" % st.name)
                gp, fty = s.field_ptr_ty(sl, st.fields)
                dp = s.op("load ptr, ptr %s" % gp)
                lp = s.op("getelementptr inbounds %s, ptr %s, i32 0, i32 1" % (ARR, gp))
                ln = s.op("load i64, ptr %s" % lp)
                a = V("arr", dp, ln, elem_ty(fty))
            else:
                a = sl if kind == "ssa" else s.load(sl)     # G5 A3: an inout parameter is a pointer view, stored through
            g = s.index_ptr(a, i)
            if a.layout != "i64":
                ty = storage_ty(a)
                z = s.op("trunc i64 %s to %s" % (v.a, ty))
                s.ins("store %s %s, ptr %s" % (ty, z, g))
            elif is_boolarr(a):  # G2 A5
                z = s.op("zext i1 %s to i8" % v.a)
                s.ins("store i8 %s, ptr %s" % (z, g))
            elif is_u8(a):
                if not s.verified:
                    ok = s.op("icmp ule i64 %s, 255" % v.a)
                    s.guard(ok, s.panic_elem(v.a), False)
                b = s.op("trunc i64 %s to i8" % v.a)
                s.ins("store i8 %s, ptr %s" % (b, g))
            else:
                s.ins("store i64 %s, ptr %s" % (v.a, g))
            s.end_temps()
        elif isinstance(st, If):
            s.begin_temps()
            c = s.ev(st.cond)
            s.end_temps()
            thn, end = s.label("then"), s.label("endif")
            els = s.label("else") if st.els else end
            s.condbr(c.a, thn, els)
            s.place(thn)
            reach = False
            if s.run_block(st.then):
                s.br(end)
                reach = True
            if st.els:
                s.place(els)
                if s.run_block(st.els):
                    s.br(end)
                    reach = True
            else:
                reach = True
            if reach:
                s.place(end)
            else:
                s.dead = True
        elif isinstance(st, While):
            cl, bl, el = s.label("wcond"), s.label("wbody"), s.label("wend")
            s.br(cl)
            s.place(cl)
            s.begin_temps()
            c = s.ev(st.cond)
            s.end_temps()
            s.condbr(c.a, bl, el)
            s.place(bl)
            if s.cost:
                s.cost_add("1")
            if s.run_block(st.body, loop=True):
                s.br(cl)
            s.place(el)
        elif isinstance(st, Return):
            s.begin_temps()
            v = V("unit")
            if st.value is not None and s.truncate_move(st.value) is not None:
                # P2 batch 4: `return truncate(x, m)` of an owned local array: the same pointer with length m moves out
                sl = s.truncate_move(st.value)
                src = s.load(sl)
                m = s.ev(st.value.args[1])
                if not s.verified:
                    ok = s.op("icmp ule i64 %s, %s" % (m.a, src.b))
                    s.guard(ok, s.panic1("pruv_panic_arith", "truncate length outside 0..len"), False)
                # P3: no runtime call: the same pointer with length m moves out and keeps the source cap, so the
                # eventual free still gets the real allocation size
                v = V("arr", src.a, m.a, src.ty, layout=src.layout, rng=src.rng, c=capof(src))
                s.ins("store ptr null, ptr %s" % sl.a)
                s.ins("store i64 0, ptr %s" % sl.b)
                s.ins("store i64 0, ptr %s" % sl.c)
            elif st.value is not None:
                val = st.value
                if isinstance(val, Var) and val.ty == STR and s.lookup(val.name)[0] == "ssa":
                    # LG9 (G-DB15-2): `return x` of a str match binder (the checker admits it only for a consuming
                    # match) is `return copy(x)`, which moves the payload out of the owned scrutinee (ret_movable)
                    val = Call(val.line, val.col, "copy", [val], STR)
                s.ret_cand = s.ret_movable(val)
                s.ret_take = s.ret_takable(val)
                v = s.ev(val, True)
                s.ret_cand = {}
                s.ret_take = set()
                if v.k == "arr" and isinstance(val, Var):
                    # moving a local array out: empty its slot so the cleanup below does not free it
                    sl = s.lookup(val.name)[1]
                    s.ins("store ptr null, ptr %s" % sl.a)
                    s.ins("store i64 0, ptr %s" % sl.b)
                    s.ins("store i64 0, ptr %s" % sl.c)
            s.end_temps()
            for sc in reversed(s.scopes):
                s.free_scope(sc)
            s.ret_moved = {}
            s.emit_ret(v)
        elif isinstance(st, Assert):
            pass
        elif isinstance(st, ExprStmt):
            s.begin_temps()
            s.ev(st.expr)
            s.end_temps()
        elif isinstance(st, Par):
            s.parallel(st)
        elif isinstance(st, Match) and is_owned_result(st.scrutinee.ty):    # LG12 E2: consuming match
            s.owned_match(st)
        elif isinstance(st, Match) and s.is_ext(st.scrutinee.ty):
            # G9 A3: the scrutinee is borrowed, never consumed; every temporary it created (a fresh scrutinee, a
            # fresh base of a field path, a fresh argument) is moved into owner slots of a scope that spans all
            # arms, so it lives until the selected arm exits (also through an early return) and is dropped once.
            s.begin_temps()
            sv = s.ev(st.scrutinee)
            held = s.temps.pop()
            s.push_scope()
            for tv in held:
                s.scopes[-1]["arrays"].append(s.hold(tv))
            s.user_match(st, sv)
            s.pop_scope()
        elif isinstance(st, Match):
            s.begin_temps()
            sv = s.ev(st.scrutinee)
            s.end_temps()
            if sv.k == "agg" and sv.ty is not None and gopt(sv.ty):  # G3: Option<X>/Result<X>, X != i64
                s.gmatch(st, sv)
                return
            if st.arms and st.arms[0].enum_name is not None:
                s.user_match(st, sv)
                return
            pos = [a for a in st.arms if a.ctor in ("Some", "Ok")][0]
            neg = [a for a in st.arms if a.ctor in ("None", "Err")][0]
            pl, nl_, end = s.label("armT"), s.label("armF"), s.label("endmatch")
            s.condbr(sv.a, pl, nl_)
            reach = False
            for arm, lbl in ((pos, pl), (neg, nl_)):
                s.place(lbl)
                bind = None
                if arm.binders:
                    bind = {arm.binders[0]: ("ssa", V("i64", sv.b))}
                if s.run_block(arm.body, bind):
                    s.br(end)
                    reach = True
            if reach:
                s.place(end)
            else:
                s.dead = True
        else:
            raise Exception("llvmgen: unsupported statement %r" % st)

    def cost_fail(s):
        s.ins("call void @pruv_panic_cost()")

    def cost_add(s, units):
        old = s.op("load i128, ptr %cost.spent")
        pair = s.op("call { i128, i1 } @llvm.uadd.with.overflow.i128(i128 %s, i128 %s)" % (old, units))
        value = s.op("extractvalue { i128, i1 } %s, 0" % pair)
        overflow = s.op("extractvalue { i128, i1 } %s, 1" % pair)
        s.guard(overflow, s.cost_fail, True)
        s.ins("store i128 %s, ptr %%cost.spent" % value)

    def cost_call(s, name, args):
        bound = s.op("call i128 @%s__pruv_cost(%s)" % (mangle(name), ", ".join(args)))
        negative = s.op("icmp slt i128 %s, 0" % bound)
        s.guard(negative, s.cost_fail, True)
        # Nonnegative i128 bound plus one always fits u128.
        units = s.op("add i128 %s, 1" % bound)
        s.cost_add(units)

    def cost_check(s):
        negative = s.op("icmp slt i128 %s, 0" % s.cost_bound)
        spent = s.op("load i128, ptr %cost.spent")
        exceeded = s.op("icmp ugt i128 %s, %s" % (spent, s.cost_bound))
        bad = s.op("or i1 %s, %s" % (negative, exceeded))
        s.guard(bad, s.cost_fail, True)

    def parallel(s, st):
        # Entry-block storage is reused only after the synchronous bridge has joined.
        jobs = s.tmp()
        jt = "[%d x %%pruv_g7_job]" % len(st.calls)
        s.allocas.append("%s = alloca %s" % (jobs, jt))
        for i, call in enumerate(st.calls):
            f = s.funcs[call.name]
            fields = []
            cost_args = []
            for p, x in zip(f.params, call.args):
                if isinstance(x, InoutArg) and inout_slot_param(p):
                    ptr = s.inout_place_ptr(x.expr)
                    fields.append(("ptr", ptr))
                    if s.cost:
                        cost_args.append("ptr %s" % ptr)
                    continue
                v = s.ev(x.expr if isinstance(x, InoutArg) else x)
                if s.cost:
                    cost_args.extend(["ptr %s" % v.a, "i64 %s" % v.b] if v.k == "arr" else
                                     ["%s %s" % (v.k, v.a)])
                if v.k == "arr":
                    fields.extend([("ptr", v.a), ("i64", v.b)])
                elif v.k == "i1":
                    fields.append(("i8", s.op("zext i1 %s to i8" % v.a)))
                else:
                    fields.append((v.k, v.a))
            if s.cost:
                s.cost_call(call.name, cost_args)
            n = len(s.m.par_types)
            ct = "%%pruv_g7_ctx_%d" % n
            cb = "@pruv_g7_cb_%d" % n
            s.m.par_types.append("%s = type { %s }" % (ct, ", ".join(t for t, _ in fields)))
            ctx = s.tmp()
            s.allocas.append("%s = alloca %s" % (ctx, ct))
            tr = ["define private void %s(ptr %%ctx) {" % cb, "entry:"]
            args = []
            for j, (ty, value) in enumerate(fields):
                ptr = s.op("getelementptr inbounds %s, ptr %s, i32 0, i32 %d" % (ct, ctx, j))
                s.ins("store %s %s, ptr %s" % (ty, value, ptr))
                tr += ["  %%p%d = getelementptr inbounds %s, ptr %%ctx, i32 0, i32 %d" % (j, ct, j),
                       "  %%v%d = load %s, ptr %%p%d" % (j, ty, j)]
                if ty == "i8":
                    tr.append("  %%b%d = icmp ne i8 %%v%d, 0" % (j, j))
                    args.append("i1 %%b%d" % j)
                else:
                    args.append("%s %%v%d" % (ty, j))
            target = "par_chk" if not s.verified and f.requires else "body"
            tr += ["  call void @%s__pruv_%s(%s)" % (mangle(f.name), target, ", ".join(args)),
                   "  ret void", "}"]
            s.m.par_trampolines.append("\n".join(tr) + "\n")
            for j, value in enumerate((cb, ctx)):
                ptr = s.op("getelementptr inbounds %s, ptr %s, i32 0, i64 %d, i32 %d" % (jt, jobs, i, j))
                s.ins("store ptr %s, ptr %s" % (value, ptr))
        s.ins("call void @pruv_g7_v1_run(ptr %s, i64 %d)" % (jobs, len(st.calls)))

    def user_match(s, st, sv):
        """match on a user enum: tag switch; payload extraction only inside the arm block; default unreachable"""
        ty = sv.ty
        tag = s.tag_of(sv, False)
        arms = [(a, s.label("arm" + a.ctor)) for a in st.arms]
        dflt, end = s.label("matchdef"), s.label("endmatch")
        # P3: payload binders of a match on an owned extended local that no arm names again may move out on a return
        own = None
        sc = st.scrutinee
        if isinstance(sc, Var) and (not s.cost or codegen.match_moves_binder(st)):   # G-DB16-1: a binder move also under cost
            kind, x = s.lookup(sc.name)
            named = set()
            for a in st.arms:
                named.update(a.binders)
                codegen._stmt_names(a.body, named)
            if kind == "slot" and x.k == "agg" and s.owned_slot(x) and sc.name not in named:
                own = x
        # an array binder moved by a let / assignment empties its field of the owner, so the owner must be dead too
        own_dead = own is not None and s.dead_after(sc.name)
        s.term("switch i64 %s, label %%%s [ %s ]" % (tag, dflt, " ".join(
            "i64 %d, label %%%s" % (s.tt.variant_index(ty, a.ctor), l) for a, l in arms)))
        s.place(dflt)
        s.term("unreachable")
        reach = False
        for a, l in arms:
            s.place(l)
            bind = {}
            for i, (name, fd) in enumerate(zip(a.binders, s.tt.fields(ty, a.ctor))):
                k = payload_index(s.tt, ty, a.ctor, i)
                bv = s.getf(sv, ty, k, fd, a.ctor, False)
                if own is not None and (s.is_ext(fd.type) or is_arr(fd.type)) and not s.fb(ty, a.ctor, fd.name):
                    bv.mv = (own, a.ctor, k, own_dead)
                bind[name] = ("ssa", bv)
            if s.run_block(a.body, bind):
                s.br(end)
                reach = True
        if reach:
            s.place(end)
        else:
            s.dead = True

    # ---------- code expressions ----------
    def range_md(s, a):
        """P3 batch 2: ", !range !N" for a load of an element of the array value a (verified builds only), else "" """
        if not s.verified or a.rng is None:
            return ""
        lo, hi = a.rng
        w = int(a.layout[1:])
        if a.layout[0] == "u":
            if not (0 <= lo and hi <= (1 << w) - 1):
                return ""
        elif not (-(1 << (w - 1)) <= lo and hi <= (1 << (w - 1)) - 1):
            return ""
        if lo > hi or hi - lo + 1 >= (1 << w):
            return ""

        def sg(x):
            x %= 1 << w
            return x - (1 << w) if x >= 1 << (w - 1) else x
        body = "i%d %d, i%d %d" % (w, sg(lo), w, sg(hi + 1))
        nodes = s.m.range_nodes
        if body not in nodes:
            nodes[body] = len(nodes)
        return ", !range !%d" % nodes[body]

    def narrow_load(s, a, ptr, target="i64"):
        ty = storage_ty(a)
        x = s.op("load %s, ptr %s%s" % (ty, ptr, s.range_md(a)))
        return s.op("%s %s %s to %s" % ("sext" if a.layout.startswith("i") else "zext", ty, x, target))

    def index_ptr(s, a, i):
        if not s.verified:
            ok = s.op("icmp ult i64 %s, %s" % (i.a, a.b))
            s.guard(ok, lambda: s.ins("call void @pruv_panic_oob(i64 %s, i64 %s)" % (i.a, a.b)), False)
        return s.op("getelementptr inbounds %s, ptr %s, i64 %s" % (storage_ty(a), a.a, i.a))  # G2 A5: bytes = u8|bool

    def resize_guards(s, src, cnt, v):
        """--no-verify guards of resize(src, cnt, v): len(src) <= cnt <= 2^40, and v <= 255 for [u8]"""
        if not s.verified:
            lo = s.op("icmp sle i64 %s, %s" % (src.b, cnt.a))
            hi = s.op("icmp ule i64 %s, 1099511627776" % cnt.a)
            ok = s.op("and i1 %s, %s" % (lo, hi))
            s.guard(ok, s.panic1("pruv_panic_arith", "resize length outside len..2^40"), False)
            if src.ty is not None and src.ty.name == "array_u8":
                ok = s.op("icmp ule i64 %s, 255" % v.a)
                s.guard(ok, s.panic_elem(v.a), False)

    def regrow_assign(s, st, sl):
        """LG32 (G-DB52-4): `x = resize(x, n, v)` on an owned local / inout array slot grows the slot's block in place
        (realloc), so the old and new blocks are never live together.  LG34 (G-LG32-3): every scalar element kind
        (u8, bool, i64 and the narrow layouts).  Returns False when the statement does not have that shape (the caller
        then uses the generic path)."""
        e = st.value
        if (s.cost or sl.k != "arr" or not isinstance(e, Call) or e.name != "resize" or len(e.args) != 3
                or not isinstance(e.args[0], Var) or e.args[0].name != st.name):
            return False
        src = s.load(sl)                    # a = ptr, b = len, c = cap
        q, cnt, nc = s.regrow_val(src, e)
        s.store(sl, V("arr", q, cnt.a, sl.ty, layout=sl.layout, rng=sl.rng, c=nc))
        return True

    def regrow_field(s, st, sl):
        """LG34 (G-LG32-3): `x.f = resize(x.f, n, v)` (the same field path on both sides, a buffer field of an extended
        struct local or `inout` struct parameter) grows the field's block in place like regrow_assign; the field keeps
        the block with cap max(n, cap).  Returns False when the statement does not have that shape."""
        e = st.value
        if (s.cost or not isinstance(e, Call) or e.name != "resize" or len(e.args) != 3
                or not s.is_ext(sl.ty) or not s.tt.is_struct(sl.ty)):
            return False
        if not codegen.same_field_path(e.args[0], st.name, st.fields):
            return False
        cty = sl.ty
        for fname in st.fields[:-1]:
            if not s.tt.is_struct(cty):
                return False
            cty = s.tt.fields(cty)[s.tt.field_index(cty, fname)].type
        if not s.tt.is_struct(cty) or s.fb(cty, None, st.fields[-1]):
            return False
        gp, fty = s.field_ptr_ty(sl, st.fields)
        if not is_arr(fty):
            return False
        src = s.unpack(s.op("load %s, ptr %s" % (ARR, gp)), fty, False)
        q, cnt, nc = s.regrow_val(src, e)
        s.ins("store %s %s, ptr %s" % (ARR, s.agg(V("arr", q, cnt.a, src.ty, c=nc)), gp))
        return True

    def regrow_val(s, src, e):
        """the in-place regrow of the owned array value src (its block, len and cap) by resize(_, n, v): the guards of
        resize, then the helper of src's element kind; returns (new block, n, new cap)"""
        cnt = s.ev(e.args[1])
        v = s.ev(e.args[2])
        s.resize_guards(src, cnt, v)
        cap = capof(src)
        if src.ty is not None and src.ty.name == "array_bool":
            z = s.op("zext i1 %s to i64" % v.a)
            s.m.resize.add("regrow_u8")
            q = s.op("call ptr @pruv_regrow_u8(ptr %s, i64 %s, i64 %s, i64 %s, i64 %s)" % (src.a, cap, src.b, cnt.a, z))
        elif src.ty is not None and src.ty.name == "array_u8":
            s.m.resize.add("regrow_u8")
            q = s.op("call ptr @pruv_regrow_u8(ptr %s, i64 %s, i64 %s, i64 %s, i64 %s)" % (src.a, cap, src.b, cnt.a, v.a))
        elif src.layout != "i64":
            s.m.resize.add("regrow_narrow")
            q = s.op("call ptr @pruv_regrow_narrow(ptr %s, i64 %s, i64 %s, i64 %s, i64 %s, i64 %d, i64 %d)"
                     % (src.a, cap, src.b, cnt.a, v.a, elem_size(src.layout), alignment(src.layout)))
        else:
            s.m.resize.add("regrow_i64")
            q = s.op("call ptr @pruv_regrow(ptr %s, i64 %s, i64 %s, i64 %s, i64 %s)" % (src.a, cap, src.b, cnt.a, v.a))
        gt = s.op("icmp sgt i64 %s, %s" % (cnt.a, cap))
        nc = s.op("select i1 %s, i64 %s, i64 %s" % (gt, cnt.a, cap))
        return q, cnt, nc

    def truncate_move(s, e):
        """P2 batch 4: the slot of x when e is `truncate(x, m)` with x an owned local array (not a parameter or binder)
        and m does not name x; None otherwise"""
        if not (not s.cost and isinstance(e, Call) and e.name == "truncate" and len(e.args) == 2
                and isinstance(e.args[0], Var)):
            return None
        names = set()
        codegen._expr_names(e.args[1], names)
        if e.args[0].name in names:
            return None
        kind, sl = s.lookup(e.args[0].name)
        if kind != "slot" or sl.k != "arr" or not s.owned_slot(sl):
            return None  # LG5: a resizable inout parameter's slot is the caller's, so it is copied, not emptied
        return sl

    def word_ptr(s, a, i, nb):
        """P2 batch 4: pointer to a[i]; --no-verify first checks 0 <= i && i + nb <= len(a) (stop as for a[i])"""
        if not s.verified:
            t = s.op("sub i64 %s, %d" % (a.b, nb))
            c1 = s.op("icmp sge i64 %s, 0" % i.a)
            c2 = s.op("icmp sle i64 %s, %s" % (i.a, t))
            ok = s.op("and i1 %s, %s" % (c1, c2))
            s.guard(ok, lambda: s.ins("call void @pruv_panic_oob(i64 %s, i64 %s)" % (i.a, a.b)), False)
        return s.op("getelementptr inbounds i8, ptr %s, i64 %s" % (a.a, i.a))

    def bswap(s, bits, x):
        s.m.bswap.add(bits)
        return s.op("call i%d @llvm.bswap.i%d(i%d %s)" % (bits, bits, bits, x))

    def byte_at(s, g, k):
        """pointer to byte k of the word at g"""
        return g if k == 0 else s.op("getelementptr inbounds i8, ptr %s, i64 %d" % (g, k))

    def load_word(s, g, nb, little):
        """Read nb bytes at g as an i64 in the requested byte order."""
        if nb in (2, 4, 8):
            x = s.op("load i%d, ptr %s, align 1" % (8 * nb, g))
            if not little:
                x = s.bswap(8 * nb, x)
            return x if nb == 8 else s.op("zext i%d %s to i64" % (8 * nb, x))
        acc = None
        for k in (range(nb - 1, -1, -1) if little else range(nb)):
            p = s.byte_at(g, k)
            b = s.op("load i8, ptr %s" % p)
            b = s.op("zext i8 %s to i64" % b)
            if acc is not None:
                acc = s.op("shl i64 %s, 8" % acc)
                b = s.op("or i64 %s, %s" % (acc, b))
            acc = b
        return acc

    def store_word(s, g, w, nb, little):
        """Store the low 8*nb bits of w at g in 8/4/2/1-byte pieces."""
        off = 0
        while off < nb:
            left = nb - off
            p = 4 if left >= 4 else 2 if left >= 2 else 1
            if nb in (1, 2, 4, 8):
                p = nb
            sh = 8 * (off if little else left - p)
            x = w if sh == 0 else s.op("lshr i64 %s, %d" % (w, sh))
            if p == 8:
                if not little:
                    x = s.bswap(64, x)
                s.ins("store i64 %s, ptr %s, align 1" % (x, s.byte_at(g, off)))
            elif p == 1:
                x = s.op("trunc i64 %s to i8" % x)
                s.ins("store i8 %s, ptr %s" % (x, s.byte_at(g, off)))
            else:
                x = s.op("trunc i64 %s to i%d" % (x, 8 * p))
                if not little:
                    x = s.bswap(8 * p, x)
                s.ins("store i%d %s, ptr %s, align 1" % (8 * p, x, s.byte_at(g, off)))
            off += p

    def checked(s, ty, name, sym, a, b):
        r = s.op("call { %s, i1 } @llvm.%s.with.overflow.%s(%s %s, %s %s)" % (ty, name, ty, ty, a, ty, b))
        v = s.op("extractvalue { %s, i1 } %s, 0" % (ty, r))
        o = s.op("extractvalue { %s, i1 } %s, 1" % (ty, r))
        if ty == "i64":
            s.guard(o, s.panic1("pruv_panic_arith", "overflow in " + sym), True)
        else:
            s.guard(o, s.panic1("pruv_panic_undefined", "i128 overflow in " + sym), True)
        return v

    def temp_arr(s, v, owned):
        if not owned:
            s.temps[-1].append(v)
        return v

    def shortcircuit(s, l, rhs, is_or):
        lcur = s.cur
        rl, el = s.label("rhs"), s.label("endsc")
        if is_or:
            s.condbr(l, el, rl)
        else:
            s.condbr(l, rl, el)
        s.place(rl)
        r = rhs()
        rcur = s.cur
        s.br(el)
        s.place(el)
        return s.op("phi i1 [ %s, %%%s ], [ %s, %%%s ]" % ("true" if is_or else "false", lcur, r, rcur))

    def ev(s, e, owned=False):
        if isinstance(e, IntLit):
            return V("i64", str(e.value))
        if isinstance(e, BoolLit):
            return V("i1", "true" if e.value else "false")
        if isinstance(e, Var):
            kind, x = s.lookup(e.name)
            v = x if kind == "ssa" else s.load(x)
            if owned and v.k == "agg" and s.is_ext(v.ty):   # G9 A3: a named extended value in an owning position
                if e.name in s.ret_cand:    # P3: moved out by the return (ret_movable)
                    if kind == "slot":
                        s.ret_moved[x.a] = None
                    else:
                        sl, vn, k, _dead = x.mv
                        prev = s.ret_moved.get(sl.a)
                        s.ret_moved[sl.a] = (vn, (prev[1] if prev else set()) | {k})
                    return v
                if is_owned_result(v.ty):
                    # LG12 E2: a Result<T, String> local in an owning position MOVES (as in the Rust output): the
                    # slot becomes zero (Err, null handle), so its scope drop frees nothing
                    if kind != "slot":
                        raise PruvError("codegen", e.line, e.col, "unsupported_type",
                                        "moving the borrowed Result<_, String> %s needs the Rust backend" % e.name,
                                        "use the Rust backend", file=getattr(e, "file", None))
                    s.ins("store %s zeroinitializer, ptr %s" % (lty(v.ty), x.a))
                    v.p = None
                    return v
                return s.clone_val(v)
            return v
        if isinstance(e, Move) and e.fields:    # LG4: move an owned buffer field out; the field becomes (null, 0, 0)
            _, x = s.lookup(e.name)
            gp, fty = s.field_ptr_ty(x, e.fields)
            old = s.op("load %s, ptr %s" % (ARR, gp))
            s.ins("store %s zeroinitializer, ptr %s" % (ARR, gp))
            return s.unpack(old, fty, False)
        if isinstance(e, Move):   # G6 A3: load the owner slot, then null it
            kind, x = s.lookup(e.name)
            if kind == "ssa" and x.k == "arr" and x.mv is not None:
                # G-DB16-1: moving an array binder of a match on an owned local takes the descriptor and empties the
                # field of the scrutinee (null, 0, 0), so the scrutinee's drop frees nothing twice
                sl, _vn, k, _dead = x.mv
                s.ins("store %s zeroinitializer, ptr %s" % (ARR, g9_gep(s, lty(sl.ty), sl.a, k)))
                return x
            v = s.load(x)
            if is_array(e.ty) or e.ty == STR:
                s.empty_array_slot(x)
            elif x.k == "agg" and s.is_ext(x.ty):
                # LG9 (G-LG8-5): an extended record owns every buffer field; the whole slot becomes zero (its drop
                # then frees nothing) and the moved value is no longer backed by the emptied slot
                s.ins("store %s zeroinitializer, ptr %s" % (lty(x.ty), x.a))
                v.p = None
            else:
                s.ins("store ptr null, ptr %s" % x.a)
            return v
        if isinstance(e, Unary):
            x = s.ev(e.operand)
            if e.op == "!":
                return V("i1", s.op("xor i1 %s, true" % x.a))
            if e.op == "~":
                return V("i64", s.op("xor i64 %s, -1" % x.a))
            if s.verified:
                return V("i64", s.op("sub nsw i64 0, %s" % x.a))
            return V("i64", s.checked("i64", "ssub", "unary -", "0", x.a))
        if isinstance(e, Binary):
            return s.ev_binary(e)
        if isinstance(e, Index):
            a = s.ev(e.array)
            i = s.ev(e.index)
            g = s.index_ptr(a, i)
            if is_boolarr(a):  # G2 A5
                x = s.op("load i8, ptr %s" % g)
                return V("i1", s.op("icmp ne i8 %s, 0" % x))
            if is_u8(a) or is_strv(a):  # G2 B2: str reads like u8
                x = s.op("load i8, ptr %s" % g)
                return V("i64", s.op("zext i8 %s to i64" % x))
            if a.layout != "i64":
                return V("i64", s.narrow_load(a, g))
            return V("i64", s.op("load i64, ptr %s%s" % (g, s.range_md(a))))
        if isinstance(e, StrLit):  # G2 B2
            nm = s.m.string(e.value.decode("utf-8"))
            ln = str(len(e.value))
            if not owned:   # P3: a borrowed literal reads the read-only global directly (nothing to free)
                return V("arr", nm, ln, STR)
            p = s.op("call ptr @pruv_copy_v2(ptr %s, i64 %s, i64 1, i64 1)" % (nm, ln))
            return s.temp_arr(V("arr", p, ln, STR), owned)
        if isinstance(e, Ctor):
            if e.ty is not None and is_owned_result(e.ty):   # LG12 E2: fresh and owned; a temporary unless taken
                return s.fresh(s.mk_octor(e), owned)
            if e.ty is not None and gopt(e.ty):  # G3
                return s.mk_gctor(e, s.ev, False)
            if e.name == "None":
                return V("opt", "false", "0")
            x = s.ev(e.arg)
            return V("opt", "true" if e.name in ("Some", "Ok") else "false", x.a)
        if isinstance(e, Call):
            return s.ev_call(e, owned)
        if isinstance(e, StructLit):
            if s.is_ext(Type(e.name)):   # G9 A3: constructor fields are owning positions; the result is fresh
                return s.fresh(s.mk_struct(e, lambda x: s.ev(x, True), False), owned)
            return s.mk_struct(e, s.ev, False)
        if isinstance(e, EnumCtor):
            if s.is_ext(Type(e.enum_name)):
                return s.fresh(s.mk_enum(e, lambda x: s.ev(x, True), False), owned)
            return s.mk_enum(e, s.ev, False)
        if isinstance(e, FieldAccess):
            if e.ty in (I64, BOOL):
                root, path = field_place(e)
                if isinstance(root, Var):
                    kind, sl = s.lookup(root.name)
                    if kind == "slot" and not s.is_ext(sl.ty) and s.tt.is_struct(sl.ty):
                        gp, fty = s.field_ptr_ty(sl, path)
                        x = s.op("load %s, ptr %s" % (st(fty), gp))
                        return s.unpack(x, fty, False)
            b = s.ev(e.base)
            ty = b.ty if b.ty is not None else e.base.ty
            i = s.tt.field_index(ty, e.field)
            fd = s.tt.fields(ty)[i]
            v = s.getf(b, ty, i, fd, None, False)
            if owned and v.k == "agg" and s.is_ext(fd.type):   # G9 A3: a field path in an owning position clones
                return s.clone_val(v)
            return v
        raise Exception("llvmgen: contract-only expression in code: %r" % e)

    def check_shift_count(s, count):
        ok = s.op("icmp ult i64 %s, 64" % count)
        s.guard(ok, s.panic1("pruv_panic_arith", "shift count outside 0..64"), False)

    def ev_binary(s, e):
        op = e.op
        if op in ("&&", "||"):
            l = s.ev(e.left)

            def rhs():
                s.begin_temps()
                r = s.ev(e.right)
                s.end_temps()
                return r.a
            return V("i1", s.shortcircuit(l.a, rhs, op == "||"))
        l = s.ev(e.left)
        r = s.ev(e.right)
        if op in BIT_OP_LLVM:
            if not s.verified and op in ("<<", ">>"):
                s.check_shift_count(r.a)
            v = s.op("%s i64 %s, %s" % (BIT_OP_LLVM[op], l.a, r.a))
            if not s.verified and op == "<<":
                back = s.op("ashr i64 %s, %s" % (v, r.a))
                bad = s.op("icmp ne i64 %s, %s" % (back, l.a))
                s.guard(bad, s.panic1("pruv_panic_arith", "overflow in <<"), True)
            return V("i64", v)
        if op in ("+", "-", "*"):
            name = {"+": "add", "-": "sub", "*": "mul"}[op]
            if s.verified:
                return V("i64", s.op("%s nsw i64 %s, %s" % (name, l.a, r.a)))
            return V("i64", s.checked("i64", "s" + name, op, l.a, r.a))
        if op in ("/", "%"):
            if not s.verified:
                c1 = s.op("icmp sgt i64 %s, 0" % r.a)
                c2 = s.op("icmp sge i64 %s, 0" % l.a)
                ok = s.op("and i1 %s, %s" % (c1, c2))
                s.guard(ok, s.panic1("pruv_panic_arith",
                                     "division outside domain: need divisor > 0 and dividend >= 0"), False)
            return V("i64", s.op("%s i64 %s, %s" % ("udiv" if op == "/" else "urem", l.a, r.a)))
        cmp = {"==": "eq", "!=": "ne", "<": "slt", "<=": "sle", ">": "sgt", ">=": "sge"}[op]
        ty = "i1" if l.k == "i1" else "i64"
        return V("i1", s.op("icmp %s %s %s, %s" % (cmp, ty, l.a, r.a)))

    def ev_call(s, e, owned):
        n, a = e.name, e.args
        if s.cost and n in ("alloc", "copy", "truncate", "resize") and not getattr(e, "pruv_cost_wrapped", False):
            import copy
            plain = copy.copy(e)
            plain.pruv_cost_wrapped = True
            plain.pruv_storage_layout = s.m.storage.layout_of(e)
            plain.pruv_storage_range = s.m.storage.range_of(e)
            result = s.ev_call(plain, owned)
            length = s.op("zext i64 %s to i128" % result.b)
            units = s.op("add i128 %s, 1" % length)
            s.cost_add(units)
            return result
        if n == "len":
            x = s.ev(a[0])
            if x.k == "coll":   # G6 A3
                return V("i64", s.g6_call(foreign_shim.len_sym(x.ty), "i64", [("ptr", x.a)]))
            return V("i64", x.b)
        if (n == "string_to_str" and n not in s.funcs and isinstance(a[0], Var) and s.lookup(a[0].name)[0] == "slot"
                and s.lookup(a[0].name)[1].k == "arr"):
            # P3 M5: string_to_str of an unboxed String local (an owned str array): move it out in a take position
            # (M3 conditions), else copy it; the other paths free the array with the scope
            s.m.g6_v2 = True
            sl = s.lookup(a[0].name)[1]
            v = s.ev(a[0])
            if not s.cost and a[0].name in s.ret_take:
                s.ret_moved[sl.a] = None            # the return's cleanup skips the slot
                return s.temp_arr(v, owned)
            if not s.cost and a[0].name in s.take_let:
                s.ins("store ptr null, ptr %s" % sl.a)
                s.ins("store i64 0, ptr %s" % sl.b)
                s.ins("store i64 0, ptr %s" % sl.c)
                return s.temp_arr(v, owned)
            p = s.op("call ptr @pruv_copy_v2(ptr %s, i64 %s, i64 1, i64 1)" % (v.a, v.b))
            return s.temp_arr(V("arr", p, v.b, STR), owned)
        if n in CATALOG and n not in CATALOG_CONTRACT_ONLY and n not in s.funcs:   # G6 A3: reserved names
            return s.catalog_call(e, owned)
        if n == "alloc":
            cnt = s.ev(a[0])        # source order in every mode: n then v
            v = s.ev(a[1])
            if not s.verified:
                ok = s.op("icmp ult i64 %s, 1099511627777" % cnt.a)
                s.guard(ok, s.panic1("pruv_panic_arith", "alloc length outside 0..2^40"), False)
            if e.ty is not None and e.ty.name == "array_bool":  # G2 A5
                z = s.op("zext i1 %s to i64" % v.a)
                p = s.op("call ptr @pruv_alloc_u8(i64 %s, i64 %s)" % (cnt.a, z))
                return s.temp_arr(V("arr", p, cnt.a, ARRAY_BOOL), owned)
            if e.ty is not None and e.ty.name == "array_u8":
                if not s.verified:
                    ok = s.op("icmp ule i64 %s, 255" % v.a)
                    s.guard(ok, s.panic_elem(v.a), False)
                if s.tail_st is not None and e is s.tail_st.value:   # P3: tail_fill
                    k = s.ev(s.tail_k)
                    s.m.tail.add("u8")
                    p = s.op("call ptr @pruv_alloc_tail_u8(i64 %s, i64 %s)" % (cnt.a, k.a))
                    return s.temp_arr(V("arr", p, cnt.a, ARRAY_U8), owned)
                p = s.op("call ptr @pruv_alloc_u8(i64 %s, i64 %s)" % (cnt.a, v.a))
                return s.temp_arr(V("arr", p, cnt.a, ARRAY_U8), owned)
            layout = getattr(e, "pruv_storage_layout", s.m.storage.layout_of(e))
            rng = getattr(e, "pruv_storage_range", s.m.storage.range_of(e))
            if layout != "i64":
                p = s.op("call ptr @pruv_alloc_narrow(i64 %s, i64 %s, i64 %d, i64 %d)" %
                         (cnt.a, v.a, elem_size(layout), alignment(layout)))
                return s.temp_arr(V("arr", p, cnt.a, layout=layout, rng=rng), owned)
            if s.tail_st is not None and e is s.tail_st.value:       # P3: tail_fill
                k = s.ev(s.tail_k)
                s.m.tail.add("i64")
                p = s.op("call ptr @pruv_alloc_tail(i64 %s, i64 %s)" % (cnt.a, k.a))
                return s.temp_arr(V("arr", p, cnt.a, rng=rng), owned)
            p = s.op("call ptr @pruv_alloc(i64 %s, i64 %s)" % (cnt.a, v.a))
            return s.temp_arr(V("arr", p, cnt.a, rng=rng), owned)
        if n == "copy":
            mv = s.copy_move(a[0])
            if mv is not None:
                return s.temp_arr(mv, owned)
            src = s.ev(a[0])
            if not s.verified:
                pass    # spec: same checks as proof mode (none)
            if src.layout != "i64":
                p = s.op("call ptr @pruv_copy_v2(ptr %s, i64 %s, i64 %d, i64 %d)" %
                         (src.a, src.b, elem_size(src.layout), alignment(src.layout)))
                return s.temp_arr(V("arr", p, src.b, src.ty, layout=src.layout, rng=src.rng), owned)
            if is_bytes(src):  # G2 A5
                p = s.op("call ptr @pruv_copy_v2(ptr %s, i64 %s, i64 1, i64 1)" % (src.a, src.b))
                return s.temp_arr(V("arr", p, src.b, src.ty, layout=src.layout, rng=src.rng), owned)
            p = s.op("call ptr @pruv_copy(ptr %s, i64 %s)" % (src.a, src.b))
            return s.temp_arr(V("arr", p, src.b, rng=src.rng), owned)
        if n == "truncate":     # the first m elements of the array as a new array
            src = s.ev(a[0])
            m = s.ev(a[1])
            if not s.verified:
                ok = s.op("icmp ule i64 %s, %s" % (m.a, src.b))
                s.guard(ok, s.panic1("pruv_panic_arith", "truncate length outside 0..len"), False)
            if (isinstance(a[0], Var) and a[0].name in s.ret_cand and s.lookup(a[0].name)[0] == "slot"
                    and s.lookup(a[0].name)[1].k == "arr"):
                # P3: an owned local array the return moves (ret_movable): the same block with length m and the source
                # cap moves out (no runtime call), as `return truncate(x, m)` does; the return's cleanup skips the slot
                s.ret_moved[s.lookup(a[0].name)[1].a] = None
                return s.temp_arr(V("arr", src.a, m.a, src.ty, layout=src.layout, rng=src.rng, c=capof(src)), owned)
            if isinstance(a[0], Var) and a[0].name in s.moving:
                # LG33 (G-LG32-5): an owned local array at its last use (move_src) keeps its block with length m; as the
                # Rust backend's truncate move, a block left more than half unused is shrunk to m (pruv_shrink_v2
                # returns the block unchanged when the target is the cap, frees it and returns NULL only when the
                # target is 0, and panics if a shrinking realloc fails, so the recorded cap is always the block's real cap)
                s.empty_array_slot(s.lookup(a[0].name)[1])
                cap = capof(src)
                half = s.op("lshr i64 %s, 1" % cap)
                cut = s.op("icmp ugt i64 %s, %s" % (half, m.a))
                nc = s.op("select i1 %s, i64 %s, i64 %s" % (cut, m.a, cap))
                s.m.shrink = True
                sz = 1 if is_bytes(src) else elem_size(src.layout)
                al = 1 if is_bytes(src) else alignment(src.layout)
                p = s.op("call ptr @pruv_shrink_v2(ptr %s, i64 %s, i64 %s, i64 %d, i64 %d)" % (src.a, cap, nc, sz, al))
                return s.temp_arr(V("arr", p, m.a, src.ty, layout=src.layout, rng=src.rng, c=nc), owned)
            if src.layout != "i64":
                p = s.op("call ptr @pruv_copy_v2(ptr %s, i64 %s, i64 %d, i64 %d)" %
                         (src.a, m.a, elem_size(src.layout), alignment(src.layout)))
                return s.temp_arr(V("arr", p, m.a, src.ty, layout=src.layout, rng=src.rng), owned)
            if is_bytes(src):
                p = s.op("call ptr @pruv_copy_v2(ptr %s, i64 %s, i64 1, i64 1)" % (src.a, m.a))
                return s.temp_arr(V("arr", p, m.a, src.ty, layout=src.layout, rng=src.rng), owned)
            p = s.op("call ptr @pruv_copy(ptr %s, i64 %s)" % (src.a, m.a))
            return s.temp_arr(V("arr", p, m.a, rng=src.rng), owned)
        if n == "resize":       # P05G: a grown copy: source order a, n, v; one allocation, prefix memcpy, tail fill
            src = s.ev(a[0])
            cnt = s.ev(a[1])
            v = s.ev(a[2])
            s.resize_guards(src, cnt, v)
            if src.ty is not None and src.ty.name == "array_bool":
                z = s.op("zext i1 %s to i64" % v.a)
                s.m.resize.add("u8")
                p = s.op("call ptr @pruv_resize_u8(ptr %s, i64 %s, i64 %s, i64 %s)" % (src.a, src.b, cnt.a, z))
                return s.temp_arr(V("arr", p, cnt.a, ARRAY_BOOL), owned)
            if src.ty is not None and src.ty.name == "array_u8":
                s.m.resize.add("u8")
                p = s.op("call ptr @pruv_resize_u8(ptr %s, i64 %s, i64 %s, i64 %s)" % (src.a, src.b, cnt.a, v.a))
                return s.temp_arr(V("arr", p, cnt.a, ARRAY_U8), owned)
            if src.layout != "i64":
                # a narrow layout: the same single pass at the element's width (prefix memcpy, tail fill)
                s.m.resize.add("narrow")
                p = s.op("call ptr @pruv_resize_narrow(ptr %s, i64 %s, i64 %s, i64 %s, i64 %d, i64 %d)" %
                         (src.a, src.b, cnt.a, v.a, elem_size(src.layout), alignment(src.layout)))
                return s.temp_arr(V("arr", p, cnt.a, src.ty, layout=src.layout, rng=src.rng), owned)
            s.m.resize.add("i64")
            p = s.op("call ptr @pruv_resize(ptr %s, i64 %s, i64 %s, i64 %s)" % (src.a, src.b, cnt.a, v.a))
            return s.temp_arr(V("arr", p, cnt.a, rng=src.rng), owned)
        if n == "is_char_boundary":  # G2 B2: s then i
            sv = s.ev(a[0])
            iv = s.ev(a[1])
            r = s.op("call i32 @pruv_str_boundary(ptr %s, i64 %s, i64 %s)" % (sv.a, sv.b, iv.a))
            return V("i1", s.op("icmp ne i32 %s, 0" % r))
        if n in BIT_LLVM:   # P2: no flags, same IR with and without --no-verify
            x = s.ev(a[0])
            y = s.ev(a[1])
            if not s.verified and n in ("wrapping_shl", "ushr"):   # P2 batch 2: check the count before shifting
                s.check_shift_count(y.a)
            return V("i64", s.op("%s i64 %s, %s" % (BIT_LLVM[n], x.a, y.a)))
        if n in ("copy_range", "copy_within"):
            within = n == "copy_within"
            overlap = within or same_place(a[0].expr, a[2])
            dv = s.ev(a[0].expr)
            iv = s.ev(a[1])
            sv = dv if overlap else s.ev(a[2])
            jv = s.ev(a[2] if within else a[3])
            cnt = s.ev(a[3] if within else a[4])
            if dv.k != "arr" or sv.k != "arr":
                raise PruvError("codegen", e.line, e.col, "unsupported_type",
                                "%s is not supported by the llvm codegen layer for this operand yet" % n,
                                "copy the elements in a loop, or use the Rust backend")
            if not s.verified:
                # P13i2a.audit-1: the destination check (i < 0 || n < 0 || i > len(dst) - n) reports (i, len(dst)),
                # then the source check (j < 0 || j > len(src) - n) reports (j, len(src))
                ta = s.op("sub i64 %s, %s" % (dv.b, cnt.a))
                c1 = s.op("icmp slt i64 %s, 0" % iv.a)
                c3 = s.op("icmp slt i64 %s, 0" % cnt.a)
                c4 = s.op("icmp sgt i64 %s, %s" % (iv.a, ta))
                o = s.op("or i1 %s, %s" % (c1, c3))
                o = s.op("or i1 %s, %s" % (o, c4))
                s.guard(o, lambda: s.ins("call void @pruv_panic_oob(i64 %s, i64 %s)" % (iv.a, dv.b)), True)
                tb = s.op("sub i64 %s, %s" % (sv.b, cnt.a))
                c2 = s.op("icmp slt i64 %s, 0" % jv.a)
                c5 = s.op("icmp sgt i64 %s, %s" % (jv.a, tb))
                o = s.op("or i1 %s, %s" % (c2, c5))
                s.guard(o, lambda: s.ins("call void @pruv_panic_oob(i64 %s, i64 %s)" % (jv.a, sv.b)), True)
            tyd, tys = storage_ty(dv), storage_ty(sv)
            if not ((is_bytes(dv) and is_bytes(sv)) or (tyd == "i64" and tys == "i64")):
                raise PruvError("codegen", e.line, e.col, "unsupported_type",
                                "%s is not supported by the llvm codegen layer for this array layout yet" % n,
                                "copy the elements in a loop, or use the Rust backend")
            pd = s.op("getelementptr inbounds %s, ptr %s, i64 %s" % (tyd, dv.a, iv.a))
            ps = s.op("getelementptr inbounds %s, ptr %s, i64 %s" % (tys, sv.a, jv.a))
            nb = cnt.a if tyd == "i8" else s.op("mul nuw nsw i64 %s, 8" % cnt.a)
            if overlap:
                s.m.memmove = True
            s.ins("call void @llvm.%s.p0.p0.i64(ptr %s, ptr %s, i64 %s, i1 false)"
                  % ("memmove" if overlap else "memcpy", pd, ps, nb))
            return V("unit")
        if n == "eq_range":  # P17a: a[i..i+n] == b[j..j+n]; source order a, i, b, j, n
            av = s.ev(a[0])
            iv = s.ev(a[1])
            bv = s.ev(a[2])
            jv = s.ev(a[3])
            cnt = s.ev(a[4])
            if av.k != "arr" or bv.k != "arr":
                raise PruvError("codegen", e.line, e.col, "unsupported_type",
                                "eq_range is not supported by the llvm codegen layer for this operand yet",
                                "compare the elements in a loop, or use the Rust backend")
            if not s.verified:
                # one combined check: i < 0 || j < 0 || n < 0 || i > len(a) - n || j > len(b) - n
                ta = s.op("sub i64 %s, %s" % (av.b, cnt.a))
                tb = s.op("sub i64 %s, %s" % (bv.b, cnt.a))
                c1 = s.op("icmp slt i64 %s, 0" % iv.a)
                c2 = s.op("icmp slt i64 %s, 0" % jv.a)
                c3 = s.op("icmp slt i64 %s, 0" % cnt.a)
                c4 = s.op("icmp sgt i64 %s, %s" % (iv.a, ta))
                c5 = s.op("icmp sgt i64 %s, %s" % (jv.a, tb))
                o = s.op("or i1 %s, %s" % (c1, c2))
                o = s.op("or i1 %s, %s" % (o, c3))
                o = s.op("or i1 %s, %s" % (o, c4))
                o = s.op("or i1 %s, %s" % (o, c5))
                s.guard(o, lambda: s.ins("call void @pruv_panic_oob(i64 %s, i64 %s)" % (iv.a, av.b)), True)
            tya, tyb = storage_ty(av), storage_ty(bv)
            if not ((is_bytes(av) and is_bytes(bv)) or (tya == "i64" and tyb == "i64")):
                # the storage plan pins eq_range operands to i64 or bytes; any other pair is not lowered
                raise PruvError("codegen", e.line, e.col, "unsupported_type",
                                "eq_range is not supported by the llvm codegen layer for this array layout yet",
                                "compare the elements in a loop, or use the Rust backend")
            if isinstance(a[4], IntLit) and cnt.a == "0":
                # an empty range is equal; alloc(0) may give a null data pointer, and memcmp(null, null, 0) is UB
                return V("i1", "true")
            pa = s.op("getelementptr inbounds %s, ptr %s, i64 %s" % (tya, av.a, iv.a))
            pb = s.op("getelementptr inbounds %s, ptr %s, i64 %s" % (tyb, bv.a, jv.a))
            sz = 1 if tya == "i8" else 8

            def eqr_mem():      # one memcmp of n * sz bytes
                s.m.memcmp = True
                nb = cnt.a if sz == 1 else s.op("mul nuw nsw i64 %s, %d" % (cnt.a, sz))
                rc = s.op("call i32 @memcmp(ptr %s, ptr %s, i64 %s)" % (pa, pb, nb))
                return s.op("icmp eq i32 %s, 0" % rc)

            def eqr_loop():     # element loop with early exit; 0 <= n is known
                pre = s.cur
                head, body, end = s.label("eqr.head"), s.label("eqr.body"), s.label("eqr.end")
                k, k1 = s.tmp(), s.tmp()
                s.br(head)
                s.place(head)
                s.ins("%s = phi i64 [ 0, %%%s ], [ %s, %%%s ]" % (k, pre, k1, body))
                d = s.op("icmp eq i64 %s, %s" % (k, cnt.a))
                s.condbr(d, end, body)
                s.place(body)
                ga = s.op("getelementptr inbounds %s, ptr %s, i64 %s" % (tya, pa, k))
                gb = s.op("getelementptr inbounds %s, ptr %s, i64 %s" % (tya, pb, k))
                x = s.op("load %s, ptr %s" % (tya, ga))
                y = s.op("load %s, ptr %s" % (tya, gb))
                ne = s.op("icmp ne %s %s, %s" % (tya, x, y))
                s.ins("%s = add nuw nsw i64 %s, 1" % (k1, k))
                s.condbr(ne, end, head)
                s.place(end)
                return s.op("phi i1 [ true, %%%s ], [ false, %%%s ]" % (head, body))

            # Measured against hand Rust slice == (work/p16/perf, docs/L3_RESTRUCTURE_SPEC.md Appendix P16/P17a):
            # a literal n is a constant-size memcmp, which LLVM expands into wide loads; a dynamic n below
            # EQR_LOOP_MAX elements takes the inline loop, otherwise memcmp.
            if isinstance(a[4], IntLit):
                r = eqr_mem()
            else:
                # the memcmp block is the taken successor: with the loop first, the in-loop memcmp
                # path measured 1.2x hand Rust (block layout around the call)
                big = s.op("icmp uge i64 %s, %d" % (cnt.a, EQR_LOOP_MAX))
                lmem, lsmall, ljoin = s.label("eqr.mem"), s.label("eqr.small"), s.label("eqr.join")
                s.condbr(big, lmem, lsmall)
                s.place(lmem)
                rm = eqr_mem()
                s.br(ljoin)
                s.place(lsmall)
                rl = eqr_loop()
                lend = s.cur
                s.br(ljoin)
                s.place(ljoin)
                r = s.op("phi i1 [ %s, %%%s ], [ %s, %%%s ]" % (rm, lmem, rl, lend))
            return V("i1", r)
        if n in ("load_be", "load_le"):   # P2 batch 4: array, then index
            arr = s.ev(a[0])
            i = s.ev(a[1])
            nb = a[2].value
            g = s.word_ptr(arr, i, nb)
            return V("i64", s.load_word(g, nb, n == "load_le"))
        if n in ("store_be", "store_le"):   # P2 batch 4: index, value, then the array
            i = s.ev(a[1])
            w = s.ev(a[2])
            arr = s.ev(a[0].expr)
            nb = a[3].value
            g = s.word_ptr(arr, i, nb)
            s.store_word(g, w.a, nb, n == "store_le")
            return V("unit")
        if n == "utf8_valid":
            b = s.ev(a[0])
            s.m.uses_utf8_valid = True
            return V("i1", s.op("call i1 @pruv_utf8_valid(ptr %s, i64 %s)" % (b.a, b.b)))
        if n == "str_from_utf8":
            b = s.ev(a[0])
            if not s.verified:
                s.m.uses_utf8_valid = s.m.uses_utf8_panic = True
                ok = s.op("call i1 @pruv_utf8_valid(ptr %s, i64 %s)" % (b.a, b.b))
                s.guard(ok, lambda: s.ins("call void @pruv_panic_utf8(i64 %s)" % b.b), False)
            if isinstance(a[0], Var) and a[0].name in s.moving:
                # LG33 (G-DB54-2): an owned local [u8] at its last use (move_src): the str takes its block and cap
                s.empty_array_slot(s.lookup(a[0].name)[1])
                return s.temp_arr(V("arr", b.a, b.b, STR, c=capof(b)), owned)
            p = s.op("call ptr @pruv_copy_v2(ptr %s, i64 %s, i64 1, i64 1)" % (b.a, b.b))
            return s.temp_arr(V("arr", p, b.b, STR), owned)
        if n == "subview":
            base, lo, hi = (s.ev(x) for x in a)
            if e.ty == STR:   # LG25 (R-6g-10): a borrowed str view for a str parameter (ptr advanced by lo)
                if not s.verified:
                    nonneg = s.op("icmp sge i64 %s, 0" % lo.a)
                    ordered = s.op("icmp sle i64 %s, %s" % (lo.a, hi.a))
                    upper = s.op("icmp sle i64 %s, %s" % (hi.a, base.b))
                if not s.verified and a[0].ty == STR:
                    # same range then boundary checks and panics as substr; the failing path re-enters pruv_substr,
                    # which panics with the matching message (it never returns there)
                    ok = s.op("and i1 %s, %s" % (nonneg, ordered))
                    ok = s.op("and i1 %s, %s" % (ok, upper))
                    s.guard(ok, lambda: s.ins("call ptr @pruv_substr(ptr %s, i64 %s, i64 %s, i64 %s)" % (base.a, base.b, lo.a, hi.a)), False)
                    b1 = s.op("call i32 @pruv_str_boundary(ptr %s, i64 %s, i64 %s)" % (base.a, base.b, lo.a))
                    b2 = s.op("call i32 @pruv_str_boundary(ptr %s, i64 %s, i64 %s)" % (base.a, base.b, hi.a))
                    c1 = s.op("icmp ne i32 %s, 0" % b1)
                    c2 = s.op("icmp ne i32 %s, 0" % b2)
                    ok2 = s.op("and i1 %s, %s" % (c1, c2))
                    s.guard(ok2, lambda: s.ins("call ptr @pruv_substr(ptr %s, i64 %s, i64 %s, i64 %s)" % (base.a, base.b, lo.a, hi.a)), False)
                elif not s.verified:
                    ok = s.op("and i1 %s, %s" % (nonneg, ordered))
                    ok = s.op("and i1 %s, %s" % (ok, upper))
                    s.guard(ok, lambda: s.ins("call void @pruv_panic_oob(i64 %s, i64 %s)" % (lo.a, base.b)), False)
                    s.m.uses_utf8_valid = s.m.uses_utf8_panic = True
                    length = s.op("sub i64 %s, %s" % (hi.a, lo.a))
                    g = s.op("getelementptr inbounds i8, ptr %s, i64 %s" % (base.a, lo.a))
                    okv = s.op("call i1 @pruv_utf8_valid(ptr %s, i64 %s)" % (g, length))
                    s.guard(okv, lambda: s.ins("call void @pruv_panic_utf8(i64 %s)" % length), False)
                length = s.op("sub i64 %s, %s" % (hi.a, lo.a))
                ptr = s.op("getelementptr inbounds i8, ptr %s, i64 %s" % (base.a, lo.a))
                return V("arr", ptr, length, STR)
            if not s.verified:
                nonneg = s.op("icmp sge i64 %s, 0" % lo.a)
                ordered = s.op("icmp sle i64 %s, %s" % (lo.a, hi.a))
                upper = s.op("icmp sle i64 %s, %s" % (hi.a, base.b))
                ok = s.op("and i1 %s, %s" % (nonneg, ordered))
                ok = s.op("and i1 %s, %s" % (ok, upper))
                s.guard(ok, lambda: s.ins("call void @pruv_panic_oob(i64 %s, i64 %s)" % (lo.a, base.b)), False)
            length = s.op("sub i64 %s, %s" % (hi.a, lo.a))
            ptr = s.op("getelementptr inbounds %s, ptr %s, i64 %s" % (storage_ty(base), base.a, lo.a))
            return V("arr", ptr, length, base.ty, layout=base.layout, rng=base.rng)
        if n == "substr":  # G2 B2: s, lo, hi in order
            sv = s.ev(a[0])
            lo = s.ev(a[1])
            hi = s.ev(a[2])
            if s.verified:
                d = s.op("sub i64 %s, %s" % (hi.a, lo.a))
                g = s.op("getelementptr inbounds i8, ptr %s, i64 %s" % (sv.a, lo.a))
                p = s.op("call ptr @pruv_copy_v2(ptr %s, i64 %s, i64 1, i64 1)" % (g, d))
            else:
                p = s.op("call ptr @pruv_substr(ptr %s, i64 %s, i64 %s, i64 %s)" % (sv.a, sv.b, lo.a, hi.a))
                d = s.op("sub i64 %s, %s" % (hi.a, lo.a))
            return s.temp_arr(V("arr", p, d, STR), owned)
        if n == "print":
            x = s.ev(a[0])
            if x.k == "i1":
                z = s.op("zext i1 %s to i64" % x.a)
                s.ins("call void @pruv_print_bool(i64 %s)" % z)
            else:
                s.ins("call void @pruv_print_i64(i64 %s)" % x.a)
            return V("unit")
        if n not in s.funcs:
            raise Exception("llvmgen: unsupported call in code: %s" % n)
        f = s.funcs[n]
        if f.foreign is not None:   # G6 A3: trusted rust fn through its bridge adapter
            return s.trusted_call(f, e, owned)
        args = []
        for p, x in zip(f.params, a):
            if isinstance(x, InoutArg) and inout_slot_param(p):
                args.append("ptr %s" % s.inout_place_ptr(x.expr))
                continue
            if s.is_ext(p.type) and p.take:   # LG9: the callee owns (and drops) fresh storage holding an owned value
                args.append("ptr %s" % s.spill(s.ev(x, True)))
                continue
            if s.is_ext(p.type):   # G9 A3: a read argument is a pointer to valid caller-owned storage
                args.append("ptr %s" % s.view_ptr(x))
                continue
            v = s.ev(x.expr if isinstance(x, InoutArg) else x, p.take)
            if v.k == "coll":   # G6 A3: opaque handle
                args.append("ptr %s" % v.a)
            elif v.k == "arr":
                args += ["ptr %s" % v.a, "i64 %s" % v.b]
                if p.take:
                    args.append("i64 %s" % capof(v))
            elif v.k == "opt":
                args.append("%s %s" % (OPT, s.agg(v)))
            elif v.k == "agg":
                args.append("%s %s" % (lty(v.ty), v.a))
            else:
                args.append("%s %s" % (v.k, v.a))
        if s.cost:
            s.cost_call(f.name, args)
        if s.m.explicit:
            # G4 A3 (docs/G4_SPEC.md 4): verified calls go to the body (requires proved); safe-mode calls go through the
            # checked wrapper only when the callee has a requires (no wrapper exists otherwise)
            callee = "@%s__pruv_%s" % (mangle(f.name), "entry" if (not s.verified and f.requires) else "body")
        else:
            callee = "@%s__pruv_%s" % (mangle(f.name), "body" if s.verified else "entry")
        if s.is_ext(f.ret):   # G9 A3: explicit output storage; the result is a fresh owned value
            outp = s.new_out(f.ret)
            s.ins("call void %s(%s)" % (callee, ", ".join(["ptr %s" % outp] + args)))
            r = s.op("load %s, ptr %s" % (lty(f.ret), outp))
            return s.fresh(V("agg", r, None, f.ret, outp), owned)
        if f.ret == UNIT:
            s.ins("call void %s(%s)" % (callee, ", ".join(args)))
            return V("unit")
        r = s.op("call %s %s(%s)" % (lty(f.ret), callee, ", ".join(args)))
        if f.ret in (I64, BOOL):
            return V(vkind(f.ret), r)
        if is_collection(f.ret):   # G6 A3: fresh owned handle; a temporary unless the caller takes it
            return s.temp_arr(V("coll", r, None, f.ret), owned)
        if is_agg(f.ret):
            return V("agg", r, None, f.ret)
        x0 = s.op("extractvalue %s %s, 0" % (lty(f.ret), r))
        x1 = s.op("extractvalue %s %s, 1" % (lty(f.ret), r))
        if is_arr(f.ret):
            x2 = s.op("extractvalue %s %s, 2" % (lty(f.ret), r))     # P3: the cap travels with the returned buffer
            return s.temp_arr(V("arr", x0, x1, elem_ty(f.ret), layout=s.m.storage.layout_of(f), rng=s.m.storage.range_of(f), c=x2), owned)
        return V("opt", x0, x1)

    # ---------- G6 A3: catalog operations and trusted rust fn through the bridge ----------
    def g6_arg(s, v):
        """bridge arguments [(type, value)] of an evaluated value: handle ptr, bool as i64 0/1, (ptr, len), i64"""
        if v.k == "coll":
            return [("ptr", v.a)]
        if v.k == "i1":
            return [("i64", s.op("zext i1 %s to i64" % v.a))]
        if v.k == "arr":
            return [("ptr", v.a), ("i64", v.b)]
        return [("i64", v.a)]

    def g6_out(s, kind):
        """fresh entry-block out slot of the given llvm type; returns its name"""
        base = "%%g%d" % s.nslot
        s.nslot += 1
        s.allocas.append("%s = alloca %s" % (base, kind))
        return base

    def catalog_call(s, e, owned):
        n = e.name
        if n in ("map_get_mut", "map_remove", "map_take", "vec_at_mut"):
            raise PruvError("codegen", e.line, e.col, "unsupported_type",
                            "%s in %s needs the Rust backend" % (n, getattr(s.f, "disp", s.f.name)),
                            "use the Rust backend", file=getattr(e, "file", None))
        cat = CATALOG[n]
        low, src = foreign_shim.inst_types(e)
        sym = foreign_shim.adapter_sym(foreign_shim.helper_name(n, src))
        args = []
        for p, x in zip(cat.params, e.args):   # arguments once, in source order; take arguments are owned values
            if isinstance(x, InoutArg):
                if p.mode == "inout" and p.type == I64:
                    args.append(("ptr", s.inout_place_ptr(x.expr)))
                    continue
                x = x.expr
            args += s.g6_arg(s.ev(x, bool(p.take)))
        if n == "vec_reserve":
            s.g6_call(sym, "void", args)
            return V("unit")
        if n in ("vec_new", "map_new", "string_from", "string_new", "vec_push", "map_insert", "string_push_str",
                 "file_open_read", "file_create", "file_read_all", "file_write_str", "file_write_bytes",
                 "proc_args_new", "proc_args_push", "proc_spawn", "proc_spawn_env", "proc_write_str", "proc_write_bytes",
                 "proc_close_stdin", "proc_read", "proc_read_all",
                 "net_connect", "net_listen", "net_accept", "net_write_str", "net_write_bytes",
                 "net_shutdown_write", "net_set_timeout", "net_read", "net_read_all", 'ev_new', 'ev_add_socket', 'ev_add_listener', 'ev_add_process', 'ev_add_timer', 'ev_wait', 'ev_read', 'ev_read_into', 'ev_proc_read_into', 'ev_write', 'ev_shutdown_write', 'ev_accept', 'ev_proc_read', 'ev_proc_write', 'ev_proc_close_stdin', 'ev_proc_wait', 'ev_proc_wait_for', 'ev_proc_write_for', 'ev_remove'):
            r = s.g6_call(sym, "ptr", args)     # a fresh box, or the very same box pointer of a consuming update
            return s.temp_arr(V("coll", r, None, e.ty), owned)
        if n in ("vec_get", "string_byte", "file_close", "file_ok", "file_readable",
                 "file_writable", "file_size", "file_byte", "proc_args_len", "proc_ok", "proc_error",
                 "proc_stdin_open", "proc_eof", "proc_size", "proc_byte",
                 "net_ok", "net_error", "net_write_open", "net_eof", "net_size", "net_byte",
                 "net_listener_ok", "net_listener_error", "net_listener_port", 'ev_ok', 'ev_error', 'ev_cap', 'ev_count', 'ev_last', 'ev_kind', 'ev_eof', 'ev_write_open', 'ev_event_slot', 'ev_event_kind', 'ev_size', 'ev_byte', 'ev_exit'):
            r = s.g6_call(sym, "i64", args)
            if e.ty == BOOL:
                return V("i1", s.op("icmp ne i64 %s, 0" % r))
            return V("i64", r)
        if n == "map_get":
            tag, val = s.g6_out("i64"), s.g6_out("i64")
            s.g6_call(sym, "void", args + [("ptr", tag), ("ptr", val)])
            t = s.op("load i64, ptr %s" % tag)
            x = s.op("load i64, ptr %s" % val)
            t1 = s.op("icmp ne i64 %s, 0" % t)
            if e.ty == OPTION:
                return V("opt", t1, x)
            x1 = s.op("icmp ne i64 %s, 0" % x)      # Option<bool>: { i1, i1 }
            T = st(e.ty)
            a0 = s.op("insertvalue %s zeroinitializer, i1 %s, 0" % (T, t1))
            return V("agg", s.op("insertvalue %s %s, i1 %s, 1" % (T, a0, x1)), None, e.ty)
        if n in ("proc_wait", "proc_kill", "net_close", "net_listener_close", "ev_close"):   # G10 proc / net A3: consume the box; (tag 1 = Ok, value) -> Result<i64>
            tag, val = s.g6_out("i64"), s.g6_out("i64")
            s.g6_call(sym, "void", args + [("ptr", tag), ("ptr", val)])
            t = s.op("load i64, ptr %s" % tag)
            x = s.op("load i64, ptr %s" % val)
            return V("opt", s.op("icmp ne i64 %s, 0" % t), x)
        if n in ("proc_bytes", "net_bytes", "ev_bytes"):   # G10 proc / net A3: a borrowed view of the public buffer, copied at once into a fresh [u8]
            s.m.g6_v2 = True
            d, l = s.g6_out("ptr"), s.g6_out("i64")
            s.g6_call(sym, "void", args + [("ptr", d), ("ptr", l)])
            dp = s.op("load ptr, ptr %s" % d)
            ln = s.op("load i64, ptr %s" % l)
            p = s.op("call ptr @pruv_copy_v2(ptr %s, i64 %s, i64 1, i64 1)" % (dp, ln))
            return s.temp_arr(V("arr", p, ln, ARRAY_U8), owned)
        if n == "string_to_str":
            s.m.g6_v2 = True
            d, l = s.g6_out("ptr"), s.g6_out("i64")
            x = e.args[0]
            mode = None     # P3 M3: take the buffer of a dying owned local String instead of copying it
            if isinstance(x, Var) and not s.cost:
                mode = "ret" if x.name in s.ret_take else ("let" if x.name in s.take_let else None)
            if mode is not None:
                s.m.g6_take = True
                c = s.g6_out("i64")     # P3: the String's capacity travels as the cap (no shrinking realloc)
                s.g6_call(foreign_shim.STRING_TAKE_SYM, "void", args + [("ptr", d), ("ptr", l), ("ptr", c)])
                sl = s.lookup(x.name)[1]
                if mode == "ret":
                    s.ret_moved[sl.a] = None        # the return's cleanup skips the slot
                else:
                    s.ins("store ptr null, ptr %s" % sl.a)      # the scope cleanup drops a null handle as nothing
                dp = s.op("load ptr, ptr %s" % d)
                ln = s.op("load i64, ptr %s" % l)
                cp = s.op("load i64, ptr %s" % c)
                return s.temp_arr(V("arr", dp, ln, STR, c=cp), owned)
            s.g6_call(foreign_shim.STRING_BYTES_SYM, "void", args + [("ptr", d), ("ptr", l)])
            dp = s.op("load ptr, ptr %s" % d)
            ln = s.op("load i64, ptr %s" % l)
            p = s.op("call ptr @pruv_copy_v2(ptr %s, i64 %s, i64 1, i64 1)" % (dp, ln))
            return s.temp_arr(V("arr", p, ln, STR), owned)
        raise Exception("llvmgen: unsupported catalog operation %s" % n)

    def trusted_call(s, f, e, owned):
        args = []
        for x in e.args:
            args += s.g6_arg(s.ev(x))
        sym = foreign_shim.adapter_sym(f.name)
        if is_collection(f.ret):   # G6b: a fresh boxed result; a temporary unless the caller takes it
            r = s.g6_call(sym, "ptr", args)
            return s.temp_arr(V("coll", r, None, f.ret), owned)
        if f.ret == UNIT:
            s.g6_call(sym, "void", args)
            return V("unit")
        r = s.g6_call(sym, "i64", args)
        if f.ret == BOOL:
            return V("i1", s.op("icmp ne i64 %s, 0" % r))
        return V("i64", r)

    # ---------- contract evaluation (requires), i128 ----------
    def bind_contract_params(s):
        s.place("start")
        s.cparams = set(p.name for p in s.f.params)
        for p in s.f.params:
            n = mangle(p.name)
            if getattr(p, "resizable", False):
                s.cenv[p.name] = s.load(s.array_slot("%%p.%s" % n, p.type))
            elif p.mode == "inout" and p.type in (I64, BOOL):
                v = s.load(Slot(vkind(p.type), "%%p.%s" % n))
                s.cenv[p.name] = s.to_c(v, p.type)
            elif is_arr(p.type):
                s.cenv[p.name] = V("arr", "%%p.%s.a" % n, "%%p.%s.n" % n, elem_ty(p.type), layout=s.m.storage.layout_of(p), rng=s.m.storage.range_of(p))
            elif p.type == BOOL:
                s.cenv[p.name] = V("i1", "%%p.%s" % n)
            elif p.type == I64:
                s.cenv[p.name] = V("i128", s.op("sext i64 %%p.%s to i128" % n))
            elif is_collection(p.type):   # G6 A3: only len(c) is evaluable in a requires on LLVM
                s.cenv[p.name] = V("coll", "%%p.%s.h" % n, None, p.type)
            elif s.is_ext(p.type):   # G9 A3: no i128 mirror; contracts read the REAL storage lazily (see cx)
                s.cenv[p.name] = V("ref", "%%p.%s" % n, None, p.type)
            elif is_agg(p.type):
                a = "%%p.%s" % n
                if p.mode == "inout":
                    a = s.op("load %s, ptr %s" % (lty(p.type), a))
                s.cenv[p.name] = s.to_c(V("agg", a, None, p.type), p.type)
            else:
                t = s.op("extractvalue %s %%p.%s, 0" % (OPT, n))
                v = s.op("extractvalue %s %%p.%s, 1" % (OPT, n))
                s.cenv[p.name] = V("opt", t, s.op("sext i64 %s to i128" % v))

    # ---------- G9 A3: contract operands over REAL values ----------
    def g9_unsupported(s, e, what):
        # LG15 (G-LG12-5): name the clause being evaluated (a kept cost clause or a runtime requires)
        clause = "kept cost clause" if getattr(s, "in_cost_clause", False) else "runtime requires"
        return PruvError("codegen", e.line, e.col, "unsupported_type",
                         "%s is not supported in a %s by the llvm codegen layer yet" % (what, clause),
                         "compare tags with is_variant and fields with variant_field, or move the check into the body")

    def tag_ptr(s, ty, p):
        return s.op("getelementptr inbounds %s, ptr %s, i32 0, i32 0" % (lty(ty), p))

    def cx(s, e):
        """pointer to the REAL (code form) storage of a contract operand of nominal or buffer type.  Nothing is
        converted eagerly, so an inactive variant or a skipped operand is never inspected.  Supported (same set as the
        Rust backend): a parameter, a field path, variant_field (partial: pruv_panic_undefined when the variant is not
        active; the tag is checked before the payload address is formed) and nullary constructors."""
        und = lambda msg: s.panic1("pruv_panic_undefined", msg)
        if isinstance(e, Var):
            if e.name not in s.cparams:
                raise s.g9_unsupported(e, "the variable %s as an aggregate operand" % e.name)
            n = mangle(e.name)
            if s.is_ext(e.ty) or any(p.name == e.name and p.mode == "inout" and is_agg(p.type) for p in s.f.params):
                return "%%p.%s" % n
            return s.spill(V("agg", "%%p.%s" % n, None, e.ty))     # a non-extended parameter is an SSA value
        if isinstance(e, FieldAccess):
            bty = e.base.ty
            bp = s.cx(e.base)
            i = s.tt.field_index(bty, e.field)
            g = s.op("getelementptr inbounds %s, ptr %s, i32 0, i32 %d" % (lty(bty), bp, i))
            if s.fb(bty, None, e.field):
                g = s.op("load ptr, ptr %s" % g)
            return g
        if isinstance(e, VariantField):
            ety = Type(e.enum_name)
            bp = s.cx(e.value)
            k = s.tt.variant_index(ety, e.variant)
            tag = s.op("load i64, ptr %s" % s.tag_ptr(ety, bp))
            ok = s.op("icmp eq i64 %s, %d" % (tag, k))
            s.guard(ok, und("variant_field expected %s::%s.%s" % (getattr(e, "src_name", e.enum_name), e.variant, e.field)),
                    False)
            i = s.tt.field_index(ety, e.field, e.variant)
            g = s.op("getelementptr inbounds %s, ptr %s, i32 0, i32 %d" % (
                lty(ety), bp, payload_index(s.tt, ety, e.variant, i)))
            if s.fb(ety, e.variant, e.field):
                g = s.op("load ptr, ptr %s" % g)
            return g
        if isinstance(e, EnumCtor) and not e.args:
            ety = Type(e.enum_name)
            x = s.op("insertvalue %s zeroinitializer, i64 %d, 0" % (st(ety), s.tt.variant_index(ety, e.variant)))
            return s.spill(V("agg", x, None, ety))
        if isinstance(e, StructLit) and not e.fields:
            return s.spill(V("agg", "zeroinitializer", None, Type(e.name)))
        if isinstance(e, (EnumCtor, StructLit)):
            # a constructor with payload is a REAL temporary of the type (code form) in entry-block storage.  An
            # extended one owns heap parts: it is dropped when the enclosing evaluation region ends (cregion).
            v = s.g9_real(e)
            base = "%%ct%d" % s.nslot
            s.nslot += 1
            s.allocas.append("%s = alloca %s" % (base, lty(v.ty)))
            s.ins("store %s %s, ptr %s" % (lty(v.ty), v.a, base))
            if s.is_ext(v.ty):
                s.cregions[-1].append((v.ty, base))
            return base
        raise s.g9_unsupported(e, "this expression as an aggregate operand")

    def cregion(s, fn):
        """run fn (a contract evaluation) and then drop the extended constructor temporaries it created, newest first.
        A region is a requires clause, a short-circuit right side or a quantifier body iteration: every temporary of
        a region is built on the straight-line path that reaches its end."""
        s.cregions.append([])
        r = fn()
        for ty, p in reversed(s.cregions.pop()):
            s.ins("call void @pruv_drop__%s(ptr %s)" % (ty.name, p))
        return r

    def g9_real(s, e, ty=None):
        """V (code form, OWNED) of the REAL value of the contract expression e of declared type ty (default e.ty).
        Constructors are built field by field in declaration order; an i64 comes from the i128 contract value through
        a checked narrowing; a named value / part is charged its weight and then deep-copied; a buffer is charged its
        length and then copied.  Same order of effects and messages as the Rust backend."""
        if isinstance(e, StructLit):
            sty = Type(e.name)
            inits = {f.name: f.value for f in e.fields}
            acc = "zeroinitializer"
            for i, fd in enumerate(s.tt.fields(sty)):
                acc = s.putf(acc, sty, i, s.g9_real(inits[fd.name], fd.type), fd, None, False)
            return V("agg", acc, None, sty)
        if isinstance(e, EnumCtor):
            ety = Type(e.enum_name)
            acc = s.op("insertvalue %s zeroinitializer, i64 %d, 0" % (st(ety), s.tt.variant_index(ety, e.variant)))
            for i, (a, fd) in enumerate(zip(e.args, s.tt.fields(ety, e.variant))):
                acc = s.putf(acc, ety, payload_index(s.tt, ety, e.variant, i), s.g9_real(a, fd.type), fd, e.variant,
                             False)
            return V("agg", acc, None, ety)
        t = ty if ty is not None else e.ty
        if isinstance(e, Ctor):
            px = t.args[0] if t is not None and t.args else I64
            if t is not None and gopt(t):
                if e.name == "None":
                    return V("agg", "zeroinitializer", None, t)
                if e.name in ("Some", "Ok"):
                    x = s.g9_real(e.arg, px)
                    acc = s.op("insertvalue %s zeroinitializer, i1 true, 0" % st(t))
                    return V("agg", s.put(acc, t, 1, x, px, False), None, t)
                x = s.g9_real(e.arg, result_error(t))
                return V("agg", s.put("zeroinitializer", t, 2, x, result_error(t), False), None, t)
            if e.name == "None":
                return V("opt", "false", "0")
            x = s.g9_real(e.arg, px if e.name != "Err" else I64)
            return V("opt", "true" if e.name in ("Some", "Ok") else "false", x.a)
        if t is None or (t.owner is None and t.name == "i64"):
            x = s.cev(e)
            tr = s.op("trunc i128 %s to i64" % x.a)
            back = s.op("sext i64 %s to i128" % tr)
            ok = s.op("icmp eq i128 %s, %s" % (back, x.a))
            s.guard(ok, s.panic1("pruv_panic_undefined", "constructor field out of i64 range"), False)
            return V("i64", tr)
        if t.owner is None and t.name == "bool":
            return V("i1", s.cev(e).a)
        if isinstance(e, (Var, FieldAccess, VariantField)) and (is_user_type(t) or t.name in ("option", "result")):
            if is_user_type(t) and not t.args and s.tt.holds_buffer(t.name):
                raise s.g9_unsupported(e, "a copy of a value holding an array or str")
            p = s.cx(e)
            s.g9_charge_weight(t, p)
            if s.is_ext(t):
                return s.clone_val(V("agg", None, None, t, p))
            return s.unpack(s.op("load %s, ptr %s" % (lty(t), p)), t, False)
        if is_arr(t):
            a = s.cev(e)
            s.spend(a.b)
            if is_bytes(a):
                np_ = s.op("call ptr @pruv_copy_v2(ptr %s, i64 %s, i64 1, i64 1)" % (a.a, a.b))
            else:
                np_ = s.op("call ptr @pruv_copy(ptr %s, i64 %s)" % (a.a, a.b))
            return V("arr", np_, a.b, elem_ty(t))
        raise s.g9_unsupported(e, "this expression as a constructor field of an aggregate operand")

    def g9_real_path(s, e):
        """e reads a part of a REAL (extended) value through its storage (cx): a variant_field of an extended value, or
        a field path below one"""
        if isinstance(e, VariantField):
            return s.is_ext(e.value.ty) or s.g9_real_path(e.value)
        if isinstance(e, FieldAccess):
            return s.is_ext(e.base.ty) or s.g9_real_path(e.base)
        return False

    def g9_charge_weight(s, t, p):
        """charge the contract budget the weight of the value of type t stored at p (one unit per nominal node); a
        type without nominal parts costs nothing"""
        for n in codegen.nominals_in(t):
            s.m.need_size.add(n)
        g9_weight(s, t, p, s.budget)

    def cload(s, e):
        """value of a field path / variant_field whose root is an extended value: a buffer (arr), an extended value
        (ref) or a scalar-like value converted to its contract form.  A whole nominal value is converted whole, so
        its weight is charged to the budget first."""
        p = s.cx(e)
        t = e.ty
        if is_arr(t):
            x = s.op("load %s, ptr %s" % (ARR, p))
            return V("arr", s.op("extractvalue %s %s, 0" % (ARR, x)), s.op("extractvalue %s %s, 1" % (ARR, x)),
                     elem_ty(t))
        if s.is_ext(t):
            return V("ref", p, None, t)
        s.g9_charge_weight(t, p)
        x = s.op("load %s, ptr %s" % (st(t), p))
        return s.to_c(s.unpack(x, t, False), t)

    def cev(s, e):
        und = lambda msg: s.panic1("pruv_panic_undefined", msg)
        if isinstance(e, IntLit):
            return V("i128", str(e.value))
        if isinstance(e, BoolLit):
            return V("i1", "true" if e.value else "false")
        if isinstance(e, Var):
            if e.name not in s.cenv:
                raise Exception("llvmgen: unsupported variable in requires: %s" % e.name)
            return s.cenv[e.name]
        if isinstance(e, Unary):
            x = s.cev(e.operand)
            if e.op == "!":
                return V("i1", s.op("xor i1 %s, true" % x.a))
            if e.op == "~":
                return V("i128", s.op("xor i128 %s, -1" % x.a))
            return V("i128", s.checked("i128", "ssub", "unary -", "0", x.a))
        if isinstance(e, Binary):
            op = e.op
            if op in ("&&", "||", "==>"):
                l = s.cev(e.left)
                if op == "==>":
                    l = V("i1", s.op("xor i1 %s, true" % l.a))
                return V("i1", s.shortcircuit(l.a, lambda: s.cregion(lambda: s.cev(e.right)).a, op != "&&"))
            if op in ("==", "!=") and s.is_ext(e.left.ty):
                # G9 A3: structural equality of buffer-free extended aggregates, visited lazily on the real values:
                # one budget unit per visited node pair, only the active variant's fields are compared
                pl = s.cx(e.left)
                pr = s.cx(e.right)
                s.m.need_eq.add(e.left.ty.name)
                eq = s.op("call i1 @pruv_ceq__%s(ptr %s, ptr %s, ptr %s)" % (e.left.ty.name, pl, pr, s.budget))
                if op == "!=":
                    eq = s.op("xor i1 %s, true" % eq)
                return V("i1", eq)
            l = s.cev(e.left)
            r = s.cev(e.right)
            if op in BIT_OP_LLVM:
                shift = op in ("<<", ">>")
                for x in (l,) if shift else (l, r):
                    t = s.op("add i128 %s, 9223372036854775808" % x.a)
                    ok = s.op("icmp ult i128 %s, 18446744073709551616" % t)
                    s.guard(ok, und("argument outside i64 in " + op), False)
                if shift:
                    ok = s.op("icmp ult i128 %s, 64" % r.a)
                    s.guard(ok, und("shift count outside 0..64 in " + op), False)
                v = s.op("%s i128 %s, %s" % (BIT_OP_LLVM[op], l.a, r.a))
                if op == "<<":
                    t = s.op("add i128 %s, 9223372036854775808" % v)
                    ok = s.op("icmp ult i128 %s, 18446744073709551616" % t)
                    s.guard(ok, und("overflow in <<"), False)
                return V("i128", v)
            if op in ("+", "-", "*"):
                name = {"+": "sadd", "-": "ssub", "*": "smul"}[op]
                return V("i128", s.checked("i128", name, op, l.a, r.a))
            if op in ("/", "%"):
                fn = "pruv_cdiv" if op == "/" else "pruv_crem"
                return V("i128", s.op("call i128 @%s(i128 %s, i128 %s)" % (fn, l.a, r.a)))
            if op in ("==", "!=") and l.k == "agg":
                eq = s.ceq(l, r, l.ty)
                if op == "!=":
                    eq = s.op("xor i1 %s, true" % eq)
                return V("i1", eq)
            if op in ("==", "!=") and l.k == "opt":
                te = s.op("icmp eq i1 %s, %s" % (l.a, r.a))
                pe = s.op("icmp eq i128 %s, %s" % (l.b, r.b))
                if e.left.ty is not None and e.left.ty.name == "option":
                    nt = s.op("xor i1 %s, true" % l.a)
                    pe = s.op("or i1 %s, %s" % (nt, pe))
                eq = s.op("and i1 %s, %s" % (te, pe))
                if op == "!=":
                    eq = s.op("xor i1 %s, true" % eq)
                return V("i1", eq)
            cmp = {"==": "eq", "!=": "ne", "<": "slt", "<=": "sle", ">": "sgt", ">=": "sge"}[op]
            return V("i1", s.op("icmp %s %s %s, %s" % (cmp, "i1" if l.k == "i1" else "i128", l.a, r.a)))
        if isinstance(e, Index):
            if e.array.ty is not None and is_collection(e.array.ty):   # G6 A3: not evaluable on LLVM yet
                raise PruvError("codegen", e.line, e.col, "unsupported_type", G6_UNSUPPORTED_MSG, G6_UNSUPPORTED_FIX)
            a = s.cev(e.array)
            i = s.cev(e.index)
            ln = s.op("zext i64 %s to i128" % a.b)
            ok = s.op("icmp ult i128 %s, %s" % (i.a, ln))
            s.guard(ok, und("array index out of range"), False)
            ii = s.op("trunc i128 %s to i64" % i.a)
            if is_boolarr(a):  # G2 A5
                g = s.op("getelementptr inbounds i8, ptr %s, i64 %s" % (a.a, ii))
                x = s.op("load i8, ptr %s" % g)
                return V("i1", s.op("icmp ne i8 %s, 0" % x))
            if is_u8(a) or is_strv(a):  # G2 B2: str reads like u8
                g = s.op("getelementptr inbounds i8, ptr %s, i64 %s" % (a.a, ii))
                x = s.op("load i8, ptr %s" % g)
                return V("i128", s.op("zext i8 %s to i128" % x))
            if a.layout != "i64":
                g = s.op("getelementptr inbounds %s, ptr %s, i64 %s" % (storage_ty(a), a.a, ii))
                return V("i128", s.narrow_load(a, g, "i128"))
            g = s.op("getelementptr inbounds i64, ptr %s, i64 %s" % (a.a, ii))
            x = s.op("load i64, ptr %s" % g)
            return V("i128", s.op("sext i64 %s to i128" % x))
        if isinstance(e, StrLit):  # G2 B2: no copy
            return V("arr", s.m.string(e.value.decode("utf-8")), str(len(e.value)), STR)
        if isinstance(e, Ctor):
            if e.ty is not None and gopt(e.ty):  # G3
                return s.mk_gctor(e, s.cev, True)
            if e.name == "None":
                return V("opt", "false", "0")
            x = s.cev(e.arg)
            return V("opt", "true" if e.name in ("Some", "Ok") else "false", x.a)
        if isinstance(e, Quant):
            return s.cquant(e)
        if isinstance(e, StructLit):
            return s.mk_struct(e, s.cev, True)
        if isinstance(e, EnumCtor):
            return s.mk_enum(e, s.cev, True)
        if isinstance(e, FieldAccess):
            if s.is_ext(e.base.ty) or s.g9_real_path(e.base):   # G9: a field of a real value, read through its storage
                return s.cload(e)
            b = s.cev(e.base)
            i = s.tt.field_index(b.ty, e.field)
            return s.get(b, b.ty, i, s.tt.fields(b.ty)[i].type, True)
        if isinstance(e, IsVariant):
            if s.is_ext(e.value.ty) or s.g9_real_path(e.value):    # G9: the tag of the real value
                ety = e.value.ty
                p = s.cx(e.value)
                tag = s.op("load i64, ptr %s" % s.tag_ptr(ety, p))
                return V("i1", s.op("icmp eq i64 %s, %d" % (tag, s.tt.variant_index(ety, e.variant))))
            b = s.cev(e.value)
            k = s.tt.variant_index(b.ty, e.variant)
            return V("i1", s.op("icmp eq i64 %s, %d" % (s.tag_of(b, True), k)))
        if isinstance(e, VariantField) and (s.is_ext(e.value.ty) or s.g9_real_path(e.value)):
            return s.cload(e)
        if isinstance(e, VariantField):
            b = s.cev(e.value)
            k = s.tt.variant_index(b.ty, e.variant)
            ok = s.op("icmp eq i64 %s, %d" % (s.tag_of(b, True), k))
            s.guard(ok, und("variant_field expected %s::%s.%s" % (e.enum_name, e.variant, e.field)), False)
            i = s.tt.field_index(b.ty, e.field, e.variant)
            fd = s.tt.fields(b.ty, e.variant)[i]
            return s.get(b, b.ty, payload_index(s.tt, b.ty, e.variant, i), fd.type, True)
        if isinstance(e, Call):
            n, a = e.name, e.args
            if n in ("file_ok", "file_readable", "file_writable", "file_size", "file_byte"):
                x = s.cev(a[0])
                args = [("ptr", x.a)]
                if n == "file_byte":
                    i = s.cev(a[1])
                    size = s.g6_call(foreign_shim.adapter_sym("pruv_g6_file_size"), "i64", args)
                    size = s.op("zext i64 %s to i128" % size)
                    ok = s.op("icmp ult i128 %s, %s" % (i.a, size))
                    s.guard(ok, und("array index out of range"), False)
                    args += [("i64", s.op("trunc i128 %s to i64" % i.a))]
                r = s.g6_call(foreign_shim.adapter_sym("pruv_g6_" + n), "i64", args)
                if e.ty == BOOL:
                    return V("i1", s.op("icmp ne i64 %s, 0" % r))
                return V("i128", s.op("zext i64 %s to i128" % r))
            if n in ("proc_args_len", "proc_ok", "proc_error", "proc_stdin_open", "proc_eof", "proc_size", "proc_byte"):
                # G10 proc A3: observers over the public state of the handle; the stream and index guards carry the
                # undefined-contract messages of the Rust source backend
                x = s.cev(a[0])
                args = [("ptr", x.a)]
                if n in ("proc_eof", "proc_size", "proc_byte"):
                    st_ = s.cev(a[1])
                    o1 = s.op("icmp eq i128 %s, 1" % st_.a)
                    o2 = s.op("icmp eq i128 %s, 2" % st_.a)
                    s.guard(s.op("or i1 %s, %s" % (o1, o2)), und("process stream outside 1..2"), False)
                    args += [("i64", s.op("trunc i128 %s to i64" % st_.a))]
                if n == "proc_byte":
                    i = s.cev(a[2])
                    size = s.g6_call(foreign_shim.adapter_sym("pruv_g6_proc_size"), "i64", args)
                    size = s.op("zext i64 %s to i128" % size)
                    s.guard(s.op("icmp ult i128 %s, %s" % (i.a, size)), und("process byte index out of range"), False)
                    args += [("i64", s.op("trunc i128 %s to i64" % i.a))]
                r = s.g6_call(foreign_shim.adapter_sym("pruv_g6_" + n), "i64", args)
                if e.ty == BOOL:
                    return V("i1", s.op("icmp ne i64 %s, 0" % r))
                if n == "proc_error":   # the error code is 0..6, never negative
                    return V("i128", s.op("sext i64 %s to i128" % r))
                return V("i128", s.op("zext i64 %s to i128" % r))
            if n in ASYNC_OBSERVERS:
                x = s.cev(a[0])
                args = [("ptr", x.a)]
                if n in ("ev_kind", "ev_eof", "ev_write_open", "ev_byte"):
                    i = s.cev(a[1])
                    bound = "ev_size" if n == "ev_byte" else "ev_cap"
                    size = s.g6_call(foreign_shim.adapter_sym("pruv_g6_" + bound), "i64", args)
                    size = s.op("zext i64 %s to i128" % size)
                    msg = "array index out of range" if n == "ev_byte" else "loop slot out of range"
                    s.guard(s.op("icmp ult i128 %s, %s" % (i.a, size)), und(msg), False)
                    args += [("i64", s.op("trunc i128 %s to i64" % i.a))]
                r = s.g6_call(foreign_shim.adapter_sym("pruv_g6_" + n), "i64", args)
                if e.ty == BOOL:
                    return V("i1", s.op("icmp ne i64 %s, 0" % r))
                return V("i128", s.op("sext i64 %s to i128" % r))
            if n in NET_OBSERVERS:
                # G10 net A3: observers over the public state of the handle; the index guard carries the
                # undefined-contract message of the Rust source backend
                x = s.cev(a[0])
                args = [("ptr", x.a)]
                if n == "net_byte":
                    i = s.cev(a[1])
                    size = s.g6_call(foreign_shim.adapter_sym("pruv_g6_net_size"), "i64", args)
                    size = s.op("zext i64 %s to i128" % size)
                    s.guard(s.op("icmp ult i128 %s, %s" % (i.a, size)), und("socket byte index out of range"), False)
                    args += [("i64", s.op("trunc i128 %s to i64" % i.a))]
                r = s.g6_call(foreign_shim.adapter_sym("pruv_g6_" + n), "i64", args)
                if e.ty == BOOL:
                    return V("i1", s.op("icmp ne i64 %s, 0" % r))
                if n == "net_error" or n == "net_listener_error":   # the error code is 0..6, never negative
                    return V("i128", s.op("sext i64 %s to i128" % r))
                return V("i128", s.op("zext i64 %s to i128" % r))
            if n == "size":   # G9 A3: structural size of the real value, checked i128 additions, one unit per node
                p = s.cx(a[0])
                s.m.need_size.add(a[0].ty.name)
                return V("i128", s.op("call i128 @pruv_csize__%s(ptr %s, ptr %s)" % (a[0].ty.name, p, s.budget)))
            if n == "len":
                x = s.cev(a[0])
                if x.k == "coll":   # G6 A3
                    ln = s.g6_call(foreign_shim.len_sym(x.ty), "i64", [("ptr", x.a)])
                    return V("i128", s.op("zext i64 %s to i128" % ln))
                return V("i128", s.op("zext i64 %s to i128" % x.b))
            if n in CATALOG_CONTRACT_ONLY:   # G6 A3: map_lookup / map_updated: not evaluable on LLVM yet
                raise PruvError("codegen", e.line, e.col, "unsupported_type", G6_UNSUPPORTED_MSG, G6_UNSUPPORTED_FIX)
            if n == "utf8_valid":
                b = s.cev(a[0])
                s.m.uses_utf8_valid = True
                return V("i1", s.op("call i1 @pruv_utf8_valid(ptr %s, i64 %s)" % (b.a, b.b)))
            if n == "utf8_valid_span":   # LG26 (R-6b-3): the --no-verify requires check; false outside the range
                b = s.cev(a[0])
                lo = s.cev(a[1])
                hi = s.cev(a[2])
                s.m.uses_utf8_valid = True
                ln = s.op("zext i64 %s to i128" % b.b)
                c1 = s.op("icmp sge i128 %s, 0" % lo.a)
                c2 = s.op("icmp sle i128 %s, %s" % (lo.a, hi.a))
                c3 = s.op("icmp sle i128 %s, %s" % (hi.a, ln))
                ok = s.op("and i1 %s, %s" % (s.op("and i1 %s, %s" % (c1, c2)), c3))
                lo64 = s.op("select i1 %s, i64 %s, i64 0" % (ok, s.op("trunc i128 %s to i64" % lo.a)))
                n64 = s.op("select i1 %s, i64 %s, i64 0" % (ok, s.op("trunc i128 %s to i64" % s.op("sub i128 %s, %s" % (hi.a, lo.a)))))
                p = s.op("getelementptr i8, ptr %s, i64 %s" % (b.a, lo64))
                v = s.op("call i1 @pruv_utf8_valid(ptr %s, i64 %s)" % (p, n64))
                return V("i1", s.op("and i1 %s, %s" % (ok, v)))
            if n == "is_char_boundary":  # G2 B2
                sv = s.cev(a[0])
                iv = s.cev(a[1])
                ln = s.op("zext i64 %s to i128" % sv.b)
                ok = s.op("icmp ule i128 %s, %s" % (iv.a, ln))
                t = s.op("trunc i128 %s to i64" % iv.a)
                u = s.op("select i1 %s, i64 %s, i64 -1" % (ok, t))
                r = s.op("call i32 @pruv_str_boundary(ptr %s, i64 %s, i64 %s)" % (sv.a, sv.b, u))
                return V("i1", s.op("icmp ne i32 %s, 0" % r))
            if n in ("load_be", "load_le"):   # P2 batch 4: 0 <= i && i + nb <= len(a) in i128, then the word
                arr = s.cev(a[0])
                i = s.cev(a[1])
                nb = a[2].value
                ln = s.op("zext i64 %s to i128" % arr.b)
                t = s.op("sub i128 %s, %d" % (ln, nb))
                c1 = s.op("icmp sge i128 %s, 0" % i.a)
                c2 = s.op("icmp sle i128 %s, %s" % (i.a, t))
                ok = s.op("and i1 %s, %s" % (c1, c2))
                s.guard(ok, und("array index out of range"), False)
                ii = s.op("trunc i128 %s to i64" % i.a)
                g = s.op("getelementptr inbounds i8, ptr %s, i64 %s" % (arr.a, ii))
                return V("i128", s.op("sext i64 %s to i128" % s.load_word(g, nb, n == "load_le")))
            if n in BIT_LLVM:   # P2: both i128 arguments must be inside i64; then the op on i128 is exact
                x = s.cev(a[0])
                y = s.cev(a[1])
                for v in (x, y):
                    t = s.op("add i128 %s, 9223372036854775808" % v.a)
                    ok = s.op("icmp ult i128 %s, 18446744073709551616" % t)
                    s.guard(ok, und("argument outside i64 in " + n), False)
                if n in ("wrapping_shl", "ushr", "wrapping_add", "wrapping_mul"):
                    # P2 batch 2/3: shifts and wrapping add/mul are not exact on i128, so truncate to i64, op, widen
                    if n in ("wrapping_shl", "ushr"):   # count in 0..64 (unsigned compare covers negatives)
                        ok = s.op("icmp ult i128 %s, 64" % y.a)
                        s.guard(ok, und("shift count outside 0..64 in " + n), False)
                    tx = s.op("trunc i128 %s to i64" % x.a)
                    ty = s.op("trunc i128 %s to i64" % y.a)
                    r = s.op("%s i64 %s, %s" % (BIT_LLVM[n], tx, ty))
                    return V("i128", s.op("sext i64 %s to i128" % r))
                return V("i128", s.op("%s i128 %s, %s" % (BIT_LLVM[n], x.a, y.a)))
            if n in ("is_some", "is_ok", "some_val", "ok_val", "err_val") and is_owned_result(a[0].ty):
                # LG15 (G-LG12-4/5): no i128 mirror of the String; the REAL storage { i1, L(T), ptr } is read lazily
                # through cx (a parameter or a field path), like the Rust backend reads its parameter
                oty = a[0].ty
                X = oty.args[0]
                # LG18 (G-LG15-2): ok_val also of a user struct / enum or a new Option / Result T that is not
                # extended (its contract mirror comes from to_c); an extended T cannot be a Result payload (type_argument)
                # LG23 (G-LG18-2): also a legacy Option<..> / one-argument Result<..> (OPT struct at payload index 1)
                okt = X in (I64, BOOL) or is_agg(X) and not s.is_ext(X) or vkind(X) == "opt"
                if n not in ("is_ok", "ok_val", "err_val") or (n == "ok_val" and not okt):
                    raise s.g9_unsupported(e, "a Result<_, String> operand")
                p = s.cx(a[0])
                tag = s.op("load i1, ptr %s" % g9_gep(s, lty(oty), p, 0))
                if n == "is_ok":
                    return V("i1", tag)
                if n == "ok_val":
                    s.guard(tag, und("ok_val of Err"), False)
                    x = s.op("load %s, ptr %s" % (lty(X), g9_gep(s, lty(oty), p, 1)))
                    if vkind(X) == "opt":
                        ot = s.op("extractvalue %s %s, 0" % (OPT, x))
                        ov = s.op("extractvalue %s %s, 1" % (OPT, x))
                        return s.to_c(V("opt", ot, ov), X)
                    return s.to_c(V(vkind(X), x, None, X), X)
                s.guard(tag, und("err_val of Ok"), True)
                return V("coll", s.op("load ptr, ptr %s" % g9_gep(s, lty(oty), p, 2)), None, result_error(oty))
            if n in ("is_some", "is_ok"):
                o = s.cev(a[0])
                if o.k == "agg":  # G3: tag of a new Option/Result
                    return V("i1", s.op("extractvalue %s %s, 0" % (st(o.ty, True), o.a)))
                return V("i1", o.a)
            if n in ("some_val", "ok_val", "err_val"):
                o = s.cev(a[0])
                if o.k == "agg":  # G3: selectors of a new Option/Result look only at the active payload
                    T = st(o.ty, True)
                    tag = s.op("extractvalue %s %s, 0" % (T, o.a))
                    if n == "err_val":
                        s.guard(tag, und("err_val of Ok"), True)
                        return s.get(o, o.ty, 2, result_error(o.ty), True)
                    s.guard(tag, und("some_val of None" if n == "some_val" else "ok_val of Err"), False)
                    return s.get(o, o.ty, 1, o.ty.args[0], True)
                if n == "err_val":
                    s.guard(o.a, und("err_val of Ok"), True)
                else:
                    s.guard(o.a, und("some_val of None" if n == "some_val" else "ok_val of Err"), False)
                return V("i128", o.b)
        raise Exception("llvmgen: unsupported expression in requires: %r" % e)

    def spend(s, n="1"):
        """consume n units (default one) of the per-entry requires iteration budget (docs/M0_CONTRACTS.md B).  The
        budget is never negative at rest and n is a non-negative length, so the subtraction cannot overflow."""
        if not hasattr(s, "budget"):
            raise Exception("llvmgen: internal error: no iteration budget slot")
        old = s.op("load i64, ptr %s" % s.budget)
        nv = s.op("sub i64 %s, %s" % (old, n))
        s.ins("store i64 %s, ptr %s" % (nv, s.budget))
        ok = s.op("icmp sge i64 %s, 0" % nv)
        s.guard(ok, s.panic1("pruv_panic_undefined", "contract iteration budget exceeded"), False)

    def csorted(s, name, q):
        """O(n) form of the sorted-precondition forall: forall k in 1..len(a): a[k-1] <= a[k]"""
        arr = s.cenv[name]
        k, r = "%%q%d.k" % q, "%%q%d.r" % q
        s.allocas.append("%s = alloca i64" % k)
        s.allocas.append("%s = alloca i1" % r)
        s.ins("store i64 1, ptr %s" % k)
        s.ins("store i1 true, ptr %s" % r)
        cl, bl, nl_, el = s.label("qc"), s.label("qb"), s.label("qn"), s.label("qe")
        s.br(cl)
        s.place(cl)
        kv = s.op("load i64, ptr %s" % k)
        c = s.op("icmp slt i64 %s, %s" % (kv, arr.b))
        s.condbr(c, bl, el)
        s.place(bl)
        s.spend()
        pk = s.op("sub i64 %s, 1" % kv)
        if arr.layout != "i64":
            g0 = s.op("getelementptr inbounds %s, ptr %s, i64 %s" % (storage_ty(arr), arr.a, pk))
            z0 = s.narrow_load(arr, g0)
            g1 = s.op("getelementptr inbounds %s, ptr %s, i64 %s" % (storage_ty(arr), arr.a, kv))
            z1 = s.narrow_load(arr, g1)
            le = s.op("icmp sle i64 %s, %s" % (z0, z1))
        elif is_u8(arr):
            g0 = s.op("getelementptr inbounds i8, ptr %s, i64 %s" % (arr.a, pk))
            g1 = s.op("getelementptr inbounds i8, ptr %s, i64 %s" % (arr.a, kv))
            x0 = s.op("load i8, ptr %s" % g0)
            x1 = s.op("load i8, ptr %s" % g1)
            z0 = s.op("zext i8 %s to i64" % x0)
            z1 = s.op("zext i8 %s to i64" % x1)
            le = s.op("icmp sle i64 %s, %s" % (z0, z1))
        else:
            g0 = s.op("getelementptr inbounds i64, ptr %s, i64 %s" % (arr.a, pk))
            g1 = s.op("getelementptr inbounds i64, ptr %s, i64 %s" % (arr.a, kv))
            x0 = s.op("load i64, ptr %s" % g0)
            x1 = s.op("load i64, ptr %s" % g1)
            le = s.op("icmp sle i64 %s, %s" % (x0, x1))
        stop = s.label("qstop")
        s.condbr(le, nl_, stop)
        s.place(stop)
        s.ins("store i1 false, ptr %s" % r)
        s.br(el)
        s.place(nl_)
        k2 = s.op("add i64 %s, 1" % kv)
        s.ins("store i64 %s, ptr %s" % (k2, k))
        s.br(cl)
        s.place(el)
        return V("i1", s.op("load i1, ptr %s" % r))

    def cfastpre(s, e, reads, lo, hi, q, el):
        """P3: branch-free prefilter of a fast_forall quantifier (codegen.fast_forall).  An empty range and a range
        whose every read is in bounds, that fits the remaining budget and whose body holds everywhere go to el with the
        result true (already stored) and the range length spent; everything else falls through to the scalar loop."""
        g = s.label("qfg")
        s.condbr(s.op("icmp sge i128 %s, %s" % (lo, hi)), el, g)
        s.place(g)
        n = s.op("sub i128 %s, %s" % (hi, lo))
        bw = s.op("sext i64 %s to i128" % s.op("load i64, ptr %s" % s.budget))
        conds = [s.op("icmp sge i128 %s, -9223372036854775808" % lo), s.op("icmp sle i128 %s, 9223372036854775807" % hi),
                 s.op("icmp sle i128 %s, %s" % (n, bw))]
        for a, c in reads:
            ln = s.op("zext i64 %s to i128" % s.cenv[a].b)
            lc = lo if c == 0 else s.op("add i128 %s, %d" % (lo, c))
            hc = hi if c == 0 else s.op("add i128 %s, %d" % (hi, c))
            conds.append(s.op("icmp sge i128 %s, 0" % lc))
            conds.append(s.op("icmp sle i128 %s, %s" % (hc, ln)))
        ok = conds[0]
        for c in conds[1:]:
            ok = s.op("and i1 %s, %s" % (ok, c))
        pre, slow = s.label("qfp"), s.label("qfs")
        s.condbr(ok, pre, slow)
        s.place(pre)
        j, acc = "%%q%d.j" % q, "%%q%d.a" % q
        s.allocas.append("%s = alloca i64" % j)
        s.allocas.append("%s = alloca i1" % acc)
        hi64 = s.op("trunc i128 %s to i64" % hi)
        s.ins("store i64 %s, ptr %s" % (s.op("trunc i128 %s to i64" % lo), j))
        s.ins("store i1 true, ptr %s" % acc)
        fc, fb, fe, ft = s.label("qfc"), s.label("qfb"), s.label("qfe"), s.label("qft")
        s.br(fc)
        s.place(fc)
        jv = s.op("load i64, ptr %s" % j)
        s.condbr(s.op("icmp slt i64 %s, %s" % (jv, hi64)), fb, fe)
        s.place(fb)
        b = s.cfast(e.body, e.var, s.op("sext i64 %s to i128" % jv), jv)
        s.ins("store i1 %s, ptr %s" % (s.op("and i1 %s, %s" % (s.op("load i1, ptr %s" % acc), b)), acc))
        s.ins("store i64 %s, ptr %s" % (s.op("add i64 %s, 1" % jv), j))
        s.br(fc)
        s.place(fe)
        s.condbr(s.op("load i1, ptr %s" % acc), ft, slow)
        s.place(ft)
        s.spend(s.op("trunc i128 %s to i64" % n))
        s.br(el)
        s.place(slow)

    def cfast(s, e, var, kv, jv, xs=None):
        """P3: a fast_forall body as straight-line IR (i1 and/or instead of short circuits, unchecked reads).
        P3-F1: xs (array name -> already loaded i64 SSA value) makes x[t] reuse that value instead of loading"""
        if isinstance(e, BoolLit):
            return "true" if e.value else "false"
        if isinstance(e, IntLit):
            return str(e.value)
        if isinstance(e, Var) and e.name == var:
            return kv
        if isinstance(e, Unary):
            return s.op("xor i1 %s, true" % s.cfast(e.operand, var, kv, jv, xs))
        if isinstance(e, Binary) and e.op in ("&&", "||", "==>"):
            l = s.cfast(e.left, var, kv, jv, xs)
            if e.op == "==>":
                l = s.op("xor i1 %s, true" % l)
            r = s.cfast(e.right, var, kv, jv, xs)
            return s.op("%s i1 %s, %s" % ("and" if e.op == "&&" else "or", l, r))
        if isinstance(e, Binary):
            cmp = {"==": "eq", "!=": "ne", "<": "slt", "<=": "sle", ">": "sgt", ">=": "sge"}[e.op]
            l = s.cfast(e.left, var, kv, jv, xs)
            return s.op("icmp %s i128 %s, %s" % (cmp, l, s.cfast(e.right, var, kv, jv, xs)))
        if isinstance(e, Index) and xs is not None:
            return s.op("sext i64 %s to i128" % xs[e.array.name])
        if isinstance(e, Index):
            a = s.cenv[e.array.name]
            ix = e.index
            i = jv if isinstance(ix, Var) else s.op("%s i64 %s, %d" % ("add" if ix.op == "+" else "sub", jv, ix.right.value))
            if e.array.ty.name == "array_u8":
                x = s.op("load i8, ptr %s" % s.op("getelementptr inbounds i8, ptr %s, i64 %s" % (a.a, i)))
                return s.op("zext i8 %s to i128" % x)
            x = s.op("load i64, ptr %s" % s.op("getelementptr inbounds i64, ptr %s, i64 %s" % (a.a, i)))
            return s.op("sext i64 %s to i128" % x)
        return s.cev(e).a

    def cquant(s, e):
        if e.kind == "forall":
            sorted_arr = codegen.match_sorted_requires(e)
            if sorted_arr is not None and not (sorted_arr in s.cenv and is_strv(s.cenv[sorted_arr])):  # G2 B2: no fast path for str
                q = s.nq
                s.nq += 1
                return s.csorted(sorted_arr, q)
        lo = s.cev(e.lo)
        hi = s.cev(e.hi)
        q = s.nq
        s.nq += 1
        k, r, h = "%%q%d.k" % q, "%%q%d.r" % q, "%%q%d.h" % q
        kind = e.kind
        s.allocas.append("%s = alloca i128" % k)
        rty = "i1" if kind in ("forall", "exists") else "i128"
        s.allocas.append("%s = alloca %s" % (r, rty))
        if kind in ("max", "min"):
            s.allocas.append("%s = alloca i1" % h)
        s.ins("store i128 %s, ptr %s" % (lo.a, k))
        if kind == "forall":
            s.ins("store i1 true, ptr %s" % r)
        elif kind == "exists":
            s.ins("store i1 false, ptr %s" % r)
        else:
            s.ins("store i128 0, ptr %s" % r)
        if kind in ("max", "min"):
            s.ins("store i1 false, ptr %s" % h)
        cl, bl, nl_, el = s.label("qc"), s.label("qb"), s.label("qn"), s.label("qe")
        reads = codegen.fast_forall(e, lambda v: v.name in s.cenv and s.cenv[v.name].k == "arr" and (
            v.ty.name == "array_u8" or s.cenv[v.name].layout == "i64"))
        if reads is not None:
            s.cfastpre(e, reads, lo.a, hi.a, q, el)
        s.br(cl)
        s.place(cl)
        kv = s.op("load i128, ptr %s" % k)
        c = s.op("icmp slt i128 %s, %s" % (kv, hi.a))
        s.condbr(c, bl, el)
        s.place(bl)
        s.spend()
        saved = s.cenv.get(e.var)
        s.cenv[e.var] = V("i128", kv)
        b = s.cregion(lambda: s.cev(e.body))
        if saved is None:
            del s.cenv[e.var]
        else:
            s.cenv[e.var] = saved
        if kind in ("forall", "exists"):
            stop = s.label("qstop")
            if kind == "forall":
                s.condbr(b.a, nl_, stop)
            else:
                s.condbr(b.a, stop, nl_)
            s.place(stop)
            s.ins("store i1 %s, ptr %s" % ("false" if kind == "forall" else "true", r))
            s.br(el)
        else:
            old = s.op("load i128, ptr %s" % r)
            if kind == "sum":
                nv = s.checked("i128", "sadd", "+", old, b.a)
            elif kind == "count":
                z = s.op("zext i1 %s to i128" % b.a)
                nv = s.op("add i128 %s, %s" % (old, z))
            else:
                hv = s.op("load i1, ptr %s" % h)
                cmp = s.op("icmp %s i128 %s, %s" % ("sgt" if kind == "max" else "slt", b.a, old))
                nh = s.op("xor i1 %s, true" % hv)
                better = s.op("or i1 %s, %s" % (nh, cmp))
                nv = s.op("select i1 %s, i128 %s, i128 %s" % (better, b.a, old))
                s.ins("store i1 true, ptr %s" % h)
            s.ins("store i128 %s, ptr %s" % (nv, r))
            s.br(nl_)
        s.place(nl_)
        k2 = s.op("add i128 %s, 1" % kv)
        s.ins("store i128 %s, ptr %s" % (k2, k))
        s.br(cl)
        s.place(el)
        if kind in ("max", "min"):
            hv = s.op("load i1, ptr %s" % h)
            s.guard(hv, s.panic1("pruv_panic_undefined", "%s over an empty range" % kind), False)
        res = s.op("load %s, ptr %s" % (rty, r))
        return V(rty, res)


def fuse_plan_ll(m, f):
    """P3-F1: codegen.fuse_map(f) when the result and the fused parameter arrays are plain i64 heap arrays, else None"""
    plan = codegen.fuse_map(f)
    if plan is None:
        return None
    if m.stack is not None and m.stack.contains(f.body[0]):
        return None
    if m.storage.layout_of(f) != "i64" or m.storage.layout_of(f.body[0].value) != "i64":
        return None
    if any(m.storage.layout_of(p) != "i64" for p in f.params if p.name in plan.arrs):
        return None
    return plan


def fuse_elem_ll(g, e, xs):
    """P3-F1: the map element expression of a fused loop with wrapping i64 arithmetic (no nsw)"""
    if isinstance(e, IntLit):
        return str(e.value)
    if isinstance(e, Index):
        return xs[e.array.name]
    l = fuse_elem_ll(g, e.left, xs)
    r = fuse_elem_ll(g, e.right, xs)
    return g.op("%s i64 %s, %s" % ({"+": "add", "-": "sub", "*": "mul"}[e.op], l, r))


def fuse_ll(g, f, plan):
    """P3-F1 (docs/P3_FUSE_DESIGN.md decision 5): the fused requires path of an element-wise map, emitted after the
    prefix requires.  A failed guard or a set flag continues at the slow path (the trailing requires) in block fzs."""
    n = g.cenv[plan.n_arr].b
    conds = [g.op("icmp eq i64 %s, %s" % (g.cenv[x].b, n)) for x in plan.arrs if x != plan.n_arr]
    need = g.op("mul i128 %s, %d" % (g.op("zext i64 %s to i128" % n), len(plan.arrs)))
    bw = g.op("sext i64 %s to i128" % g.op("load i64, ptr %s" % g.budget))
    conds.append(g.op("icmp sle i128 %s, %s" % (need, bw)))
    ok = conds[0]
    for c in conds[1:]:
        ok = g.op("and i1 %s, %s" % (ok, c))
    pre, slow = g.label("fzp"), g.label("fzs")
    g.condbr(ok, pre, slow)
    g.place(pre)
    p = g.op("call ptr @pruv_alloc(i64 %s, i64 0)" % n)
    q = g.nq
    g.nq += 1
    j, bad = "%%q%d.j" % q, "%%q%d.f" % q
    g.allocas.append("%s = alloca i64" % j)
    g.allocas.append("%s = alloca i1" % bad)
    g.ins("store i64 0, ptr %s" % j)
    g.ins("store i1 false, ptr %s" % bad)
    fc, fb, fe, fr, ff = g.label("fzc"), g.label("fzb"), g.label("fze"), g.label("fzr"), g.label("fzf")
    g.br(fc)
    g.place(fc)
    jv = g.op("load i64, ptr %s" % j)
    g.condbr(g.op("icmp slt i64 %s, %s" % (jv, n)), fb, fe)
    g.place(fb)
    xs = {}
    for x in plan.arrs:
        xs[x] = g.op("load i64, ptr %s" % g.op("getelementptr inbounds i64, ptr %s, i64 %s" % (g.cenv[x].a, jv)))
    kv = g.op("sext i64 %s to i128" % jv)
    acc = g.op("load i1, ptr %s" % bad)
    for r in f.requires[plan.start:]:
        pv = g.cfast(r.body, r.var, kv, jv, xs)
        acc = g.op("or i1 %s, %s" % (acc, g.op("xor i1 %s, true" % pv)))
    g.ins("store i1 %s, ptr %s" % (acc, bad))
    ev = fuse_elem_ll(g, plan.elem, xs)
    g.ins("store i64 %s, ptr %s" % (ev, g.op("getelementptr inbounds i64, ptr %s, i64 %s" % (p, jv))))
    g.ins("store i64 %s, ptr %s" % (g.op("add i64 %s, 1" % jv), j))
    g.br(fc)
    g.place(fe)
    g.condbr(g.op("load i1, ptr %s" % bad), ff, fr)
    g.place(fr)
    a = g.op("insertvalue %s undef, ptr %s, 0" % (ARR, p))
    a = g.op("insertvalue %s %s, i64 %s, 1" % (ARR, a, n))
    g.term("ret %s %s" % (ARR, g.op("insertvalue %s %s, i64 %s, 2" % (ARR, a, n))))     # P3: cap == len
    g.place(ff)
    g.ins("call void @pruv_free(ptr %s, i64 %s)" % (p, n))
    g.br(slow)
    g.place(slow)


def entry_of(m, funcs, f, trust, budget=1 << 30, cost=False, verified=False):
    g = FnGen(m, funcs, f, True)
    params = ir_params(f, m.tt)
    rt = ir_ret(m.tt, f)    # G9 A3: "void" for an extended result (output storage is the leading `ptr %out`)
    if trust and not cost:
        g.place("start")
    else:
        g.bind_contract_params()
        g.budget = "%req.budget"
        g.allocas.append("%s = alloca i64" % g.budget)
        g.ins("store i64 %d, ptr %s" % (budget, g.budget))
        plan = fuse_plan_ll(m, f) if verified and not trust and not cost else None   # P3-F1: verified checked entries
        for k, r in enumerate(([] if trust else f.requires)):
            if plan is not None and k == plan.start:
                fuse_ll(g, f, plan)
            c = g.cregion(lambda r=r: g.cev(r))
            msg_fn = m.string(getattr(f, "disp", f.name))   # G4: the source-qualified identity, not the stem
            msg_ex = m.string(show(r))
            g.guard(c.a, lambda a=msg_fn, b=msg_ex: g.ins("call void @pruv_panic_requires(ptr %s, ptr %s)" % (a, b)),
                    False)
    if cost:
        g.in_cost_clause = True     # LG15 (G-LG12-5): g9_unsupported names the cost clause
        value = g.cregion(lambda: g.cev(f.cost))
        g.term("ret i128 %s" % value.a)
        params = [(t, n) for t, n in params if n != "%out"]
        return g.render("define internal i128 @%s__pruv_cost(%s)" % (mangle(f.name), sig_text(params, f)))
    args = ", ".join("%s %s" % (t, n) for t, n in params)
    callee = "@%s__pruv_body" % mangle(f.name)
    if f.ret == UNIT or rt == "void":
        g.ins("call void %s(%s)" % (callee, args))
        g.term("ret void")
    else:
        r = g.op("call %s %s(%s)" % (rt, callee, args))
        g.term("ret %s %s" % (rt, r))
    return g.render("define internal %s @%s__pruv_entry(%s)" % (rt, mangle(f.name), sig_text(params, f)))


# ======================================================================================================================
# G9 A3: recursive values and aggregate buffer fields (docs/G9_SPEC.md 4, 6, 7, 13)
#
# Design note.
#  Layout.  An ordinary acyclic aggregate stays inline (%pruv_t_Name, declaration order).  A field whose type is in the
#    same recursive type SCC as its container (prog.g9["boxed"]) is an opaque `ptr` to an independently allocated node
#    holding one %pruv_t_Child; buffers ([i64], [u8], [bool], str) are { ptr, i64 }.  An enum is { i64 tag, every
#    variant's payload slots }: inactive pointers are null, inactive scalar storage is zero, and only the ACTIVE payload
#    is ever loaded, cloned, compared, sized or dropped.  Node size and alignment come from the target's own layout
#    (getelementptr-from-null, evaluated by the backend), never from Python host sizes.
#  Values.  Code evaluates an extended aggregate as an SSA value that is a shallow copy of its storage.  Whether a value
#    OWNS its resources is decided syntactically, as in the Rust backend: a constructor or a call result is fresh; a
#    variable, a parameter, a match binder or a field path is a borrowed VIEW.  A view is never dropped.
#  Ownership.  Owners are local slots (dropped, in reverse order, when their scope ends and on `return`), fresh
#    temporaries (dropped at the end of the statement; a fresh match scrutinee, and everything created while evaluating
#    it, is moved into a scope that spans all arms), constructor fields and the output storage of a result.  An owning
#    position (let, assignment, constructor field, return, field replacement) receives a fresh value as is and CLONES a
#    view.  An assignment evaluates its right side and takes ownership of the result before the old value is dropped.
#  Calls.  An extended read argument is `ptr` to valid caller-owned storage (a slot, a parameter, a node, or a spilled
#    copy of a fresh temporary that lives through the call).  An extended result is written through the leading
#    `ptr %out`.  Extended signatures are internal (no ABI wrapper, no binding).  Read pointer parameters are
#    `noalias nocapture readonly`: the same subtree may be passed twice, which is sound because neither writes it.
#  Helpers (stable names, SCC-condensed declaration order): pruv_drop__T(ptr) releases the owned parts of the value at
#    the pointer, children first, but never the root storage itself; pruv_clone__T(ptr src, ptr dst) writes a deep copy;
#    pruv_box__T / pruv_dropnode__T / pruv_clonenode__T allocate, release and clone a node (pruv_node_alloc_v1 /
#    pruv_node_free_v1); pruv_csize__T and pruv_ceq__T implement `size` and `==` in runtime contracts (checked i128
#    addition, one budget unit per visited node, same messages as the Rust backend helpers).
# ======================================================================================================================
def g9_gep(g, T, p, idx):
    return g.op("getelementptr inbounds %s, ptr %s, i32 0, i32 %d" % (T, p, idx))


def g9_variants(d):
    """[(variant name or None, tag or None, fields)] in declaration order"""
    if isinstance(d, StructDecl):
        return [(None, None, d.fields)]
    return [(v.name, i, v.fields) for i, v in enumerate(d.variants)]


def g9_sidx(tt, ty, vn, i):
    """storage index of field i of variant vn (a struct: vn is None) inside the aggregate"""
    return i if vn is None else payload_index(tt, ty, vn, i)


def g9_each_variant(g, ty, d, base, vs, emit):
    """emit(variant name, fields) for the variants in vs.  A struct runs it inline.  An enum switches on the tag of
    the value at `base` and continues, on every path, at a join label that is left as the current block."""
    if isinstance(d, StructDecl):
        for vn, _t, fs in vs:
            emit(vn, fs)
        return
    if not vs:
        return
    tag = g.op("load i64, ptr %s" % g.tag_ptr(ty, base))
    done = g.label("gdone")
    lbls = [g.label("gv") for _ in vs]
    g.term("switch i64 %s, label %%%s [ %s ]" % (tag, done, " ".join(
        "i64 %d, label %%%s" % (t, l) for (_vn, t, _fs), l in zip(vs, lbls))))
    for (vn, _t, fs), l in zip(vs, lbls):
        g.place(l)
        emit(vn, fs)
        g.br(done)
    g.place(done)


def g9_node_layout(g, T):
    """(size, align) SSA names of the node type T computed by the backend from the target layout"""
    z = g.op("getelementptr %s, ptr null, i32 1" % T)
    size = g.op("ptrtoint ptr %s to i64" % z)
    a = g.op("getelementptr { i8, %s }, ptr null, i32 0, i32 1" % T)
    align = g.op("ptrtoint ptr %s to i64" % a)
    return size, align


def g9_drop_text(m, tt, d):
    ty = Type(d.name)
    T = lty(ty)
    g = FnGen(m, {}, None, True)
    g.place("start")

    def emit(vn, fs):
        for i, f in enumerate(fs):
            ft = f.type
            if not (is_arr(ft) or g.is_ext(ft)):
                continue
            gp = g9_gep(g, T, "%v", g9_sidx(tt, ty, vn, i))
            if g.fb(ty, vn, f.name):       # a boxed child: its contents first, then its node
                c = g.op("load ptr, ptr %s" % gp)
                g.ins("call void @pruv_dropnode__%s(ptr %s)" % (ft.name, c))
            elif is_arr(ft):
                x = g.op("load %s, ptr %s" % (ARR, gp))
                g.free_val(g.unpack(x, ft, False))      # P3: freed with its cap
            else:                          # an inline extended child (not a node)
                g.ins("call void @pruv_drop__%s(ptr %s)" % (ft.name, gp))
    vs = [(vn, t, fs) for vn, t, fs in g9_variants(d) if any(is_arr(f.type) or g.is_ext(f.type) for f in fs)]
    g9_each_variant(g, ty, d, "%v", vs, emit)
    g.term("ret void")
    return g.render("define internal void @pruv_drop__%s(ptr %%v)" % d.name)


def g9_clone_text(m, tt, d):
    ty = Type(d.name)
    T = lty(ty)
    g = FnGen(m, {}, None, True)
    g.place("start")
    v0 = g.op("load %s, ptr %%src" % T)      # shallow copy: scalars, inactive null/zero slots; owners are fixed below
    g.ins("store %s %s, ptr %%dst" % (T, v0))

    def emit(vn, fs):
        for i, f in enumerate(fs):
            ft = f.type
            if not (is_arr(ft) or g.is_ext(ft)):
                continue
            idx = g9_sidx(tt, ty, vn, i)
            gs = g9_gep(g, T, "%src", idx)
            gd = g9_gep(g, T, "%dst", idx)
            if g.fb(ty, vn, f.name):
                c = g.op("load ptr, ptr %s" % gs)
                nb = g.op("call ptr @pruv_clonenode__%s(ptr %s)" % (ft.name, c))
                g.ins("store ptr %s, ptr %s" % (nb, gd))
            elif is_arr(ft):
                x = g.op("load %s, ptr %s" % (ARR, gs))
                p = g.op("extractvalue %s %s, 0" % (ARR, x))
                n = g.op("extractvalue %s %s, 1" % (ARR, x))
                if is_bytes(V("arr", p, n, elem_ty(ft))):
                    np_ = g.op("call ptr @pruv_copy_v2(ptr %s, i64 %s, i64 1, i64 1)" % (p, n))
                else:
                    np_ = g.op("call ptr @pruv_copy(ptr %s, i64 %s)" % (p, n))
                a1 = g.op("insertvalue %s %s, ptr %s, 0" % (ARR, x, np_))
                a2 = g.op("insertvalue %s %s, i64 %s, 2" % (ARR, a1, n))    # P3: the new block holds exactly len elements
                g.ins("store %s %s, ptr %s" % (ARR, a2, gd))
            else:
                g.ins("call void @pruv_clone__%s(ptr %s, ptr %s)" % (ft.name, gs, gd))
    vs = [(vn, t, fs) for vn, t, fs in g9_variants(d) if any(is_arr(f.type) or g.is_ext(f.type) for f in fs)]
    g9_each_variant(g, ty, d, "%src", vs, emit)
    g.term("ret void")
    return g.render("define internal void @pruv_clone__%s(ptr %%src, ptr %%dst)" % d.name)


def g9_node_texts(m, tt, d):
    """box / dropnode / clonenode of a type that is the target of a boxed edge"""
    n, T = d.name, lty(Type(d.name))
    out = []
    g = FnGen(m, {}, None, True)
    g.place("start")
    size, align = g9_node_layout(g, T)
    p = g.op("call ptr @pruv_node_alloc_v1(i64 %s, i64 %s)" % (size, align))
    g.ins("store %s %%v, ptr %s" % (T, p))
    g.term("ret ptr %s" % p)
    out.append(g.render("define internal ptr @pruv_box__%s(%s %%v)" % (n, T)))
    g = FnGen(m, {}, None, True)
    g.place("start")
    g.ins("call void @pruv_drop__%s(ptr %%n)" % n)
    size, align = g9_node_layout(g, T)
    g.ins("call void @pruv_node_free_v1(ptr %%n, i64 %s, i64 %s)" % (size, align))
    g.term("ret void")
    out.append(g.render("define internal void @pruv_dropnode__%s(ptr %%n)" % n))
    g = FnGen(m, {}, None, True)
    g.place("start")
    size, align = g9_node_layout(g, T)
    p = g.op("call ptr @pruv_node_alloc_v1(i64 %s, i64 %s)" % (size, align))
    g.ins("call void @pruv_clone__%s(ptr %%src, ptr %s)" % (n, p))
    g.term("ret ptr %s" % p)
    out.append(g.render("define internal ptr @pruv_clonenode__%s(ptr %%src)" % n))
    return out


def g9_walks(t):
    """size / equality walk into t: a nominal value, or Option / Result of one"""
    if gopt(t):
        return g9_walks(t.args[0]) or (t.name == "result" and g9_walks(result_error(t)))
    return is_user_type(t)


def g9_weight(g, t, p, bud="%bud"):
    """i128 SSA value of weight(value stored at p) (G9_SPEC 3), or None when it is statically 0.  Some / Ok weigh their
    payload, None / Err 0; a nominal value weighs its size (one budget unit per node, inside pruv_csize).  bud is the
    budget pointer the size helper charges."""
    if gopt(t) and t.name == "result" and g9_walks(result_error(t)):    # LG12: Ok weighs X, Err weighs E
        X, E = t.args[0], result_error(t)
        T = st(t)
        tag = g.op("load i1, ptr %s" % g9_gep(g, T, p, 0))
        lcur, yes, no, end = g.cur, g.label("gwy"), g.label("gwn"), g.label("gwe")
        g.condbr(tag, yes, no)
        g.place(yes)
        w = g9_weight(g, X, g9_gep(g, T, p, 1), bud) or "0"
        wcur = g.cur
        g.br(end)
        g.place(no)
        we = g9_weight(g, E, g9_gep(g, T, p, 2), bud) or "0"
        ecur = g.cur
        g.br(end)
        g.place(end)
        return g.op("phi i128 [ %s, %%%s ], [ %s, %%%s ]" % (w, wcur, we, ecur))
    if gopt(t):
        X = t.args[0]
        if not g9_walks(X):
            return None
        T = st(t)
        tag = g.op("load i1, ptr %s" % g9_gep(g, T, p, 0))
        pl = g9_gep(g, T, p, 1)
        lcur, yes, end = g.cur, g.label("gwy"), g.label("gwe")
        g.condbr(tag, yes, end)
        g.place(yes)
        w = g9_weight(g, X, pl, bud)
        wcur = g.cur
        g.br(end)
        g.place(end)
        return g.op("phi i128 [ 0, %%%s ], [ %s, %%%s ]" % (lcur, w, wcur))
    if is_user_type(t):
        return g.op("call i128 @pruv_csize__%s(ptr %s, ptr %s)" % (t.name, p, bud))
    return None


def g9_size_text(m, tt, d):
    """pruv_csize__T: 1 + the weights of the ACTIVE fields, checked i128 additions in field order, one budget unit"""
    ty = Type(d.name)
    T = lty(ty)
    g = FnGen(m, {}, None, True)
    g.place("start")
    g.budget = "%bud"
    g.spend()

    def fold(vn, fs):
        acc = "1"
        for i, f in enumerate(fs):
            if not g9_walks(f.type):
                continue
            gp = g9_gep(g, T, "%v", g9_sidx(tt, ty, vn, i))
            if g.fb(ty, vn, f.name):
                gp = g.op("load ptr, ptr %s" % gp)
            acc = g.checked("i128", "sadd", "+", acc, g9_weight(g, f.type, gp))
        return acc
    if isinstance(d, StructDecl):
        g.term("ret i128 %s" % fold(None, d.fields))
    else:
        g.allocas.append("%res = alloca i128")
        g.ins("store i128 1, ptr %res")
        vs = [(vn, t, fs) for vn, t, fs in g9_variants(d) if any(g9_walks(f.type) for f in fs)]
        g9_each_variant(g, ty, d, "%v", vs, lambda vn, fs: g.ins("store i128 %s, ptr %%res" % fold(vn, fs)))
        g.term("ret i128 %s" % g.op("load i128, ptr %res"))
    return g.render("define internal i128 @pruv_csize__%s(ptr %%v, ptr %%bud)" % d.name)


def g9_eq_val(g, t, pa, pb):
    """i1 SSA value: structural equality of the values stored at pa and pb (type t)"""
    n = t.name
    if gopt(t):     # Option<X> / Result<X>: equal tags, then the active payload only
        T = st(t)
        X = t.args[0]
        ta = g.op("load i1, ptr %s" % g9_gep(g, T, pa, 0))
        tb = g.op("load i1, ptr %s" % g9_gep(g, T, pb, 0))
        te = g.op("icmp eq i1 %s, %s" % (ta, tb))
        lcur, l1, lend = g.cur, g.label("gqa"), g.label("gqe")
        g.condbr(te, l1, lend)
        g.place(l1)
        l2 = g.label("gqb")
        if n == "option":
            g.condbr(ta, l2, lend)          # None == None
            g.place(l2)
            e = g9_eq_val(g, X, g9_gep(g, T, pa, 1), g9_gep(g, T, pb, 1))
            l2cur = g.cur
            g.br(lend)
            g.place(lend)
            return g.op("phi i1 [ false, %%%s ], [ true, %%%s ], [ %s, %%%s ]" % (lcur, l1, e, l2cur))
        l3 = g.label("gqc")
        g.condbr(ta, l2, l3)
        g.place(l2)
        e = g9_eq_val(g, X, g9_gep(g, T, pa, 1), g9_gep(g, T, pb, 1))
        l2cur = g.cur
        g.br(lend)
        g.place(l3)
        if result_error(t) == I64:
            ea = g.op("load i64, ptr %s" % g9_gep(g, T, pa, 2))
            eb = g.op("load i64, ptr %s" % g9_gep(g, T, pb, 2))
            e3 = g.op("icmp eq i64 %s, %s" % (ea, eb))
        else:   # LG12: the error payloads of type E
            e3 = g9_eq_val(g, result_error(t), g9_gep(g, T, pa, 2), g9_gep(g, T, pb, 2))
        l3cur = g.cur
        g.br(lend)
        g.place(lend)
        return g.op("phi i1 [ false, %%%s ], [ %s, %%%s ], [ %s, %%%s ]" % (lcur, e, l2cur, e3, l3cur))
    if n in ("option", "result"):   # legacy Option<i64> / Result<i64>: { i1, i64 }
        la = g.op("load %s, ptr %s" % (OPT, pa))
        lb = g.op("load %s, ptr %s" % (OPT, pb))
        ta, tb = g.op("extractvalue %s %s, 0" % (OPT, la)), g.op("extractvalue %s %s, 0" % (OPT, lb))
        va, vb = g.op("extractvalue %s %s, 1" % (OPT, la)), g.op("extractvalue %s %s, 1" % (OPT, lb))
        te = g.op("icmp eq i1 %s, %s" % (ta, tb))
        pe = g.op("icmp eq i64 %s, %s" % (va, vb))
        if n == "option":
            pe = g.op("or i1 %s, %s" % (g.op("xor i1 %s, true" % ta), pe))
        return g.op("and i1 %s, %s" % (te, pe))
    if n == "i64" or n == "bool":
        ty = "i64" if n == "i64" else "i1"
        return g.op("icmp eq %s %s, %s" % (ty, g.op("load %s, ptr %s" % (ty, pa)), g.op("load %s, ptr %s" % (ty, pb))))
    if is_user_type(t):
        return g.op("call i1 @pruv_ceq__%s(ptr %s, ptr %s, ptr %%bud)" % (n, pa, pb))
    raise Exception("llvmgen: internal error: no structural equality for type %s" % n)


def g9_eq_text(m, tt, d):
    """pruv_ceq__T: one budget unit, then equal tags and the fields of the active variant, left to right"""
    ty = Type(d.name)
    T = lty(ty)
    g = FnGen(m, {}, None, True)
    g.place("start")
    g.budget = "%bud"
    g.spend()
    fail = g.label("ceqf")

    def conj(vn, fs):
        for i, f in enumerate(fs):
            idx = g9_sidx(tt, ty, vn, i)
            pa, pb = g9_gep(g, T, "%a", idx), g9_gep(g, T, "%b", idx)
            if g.fb(ty, vn, f.name):
                pa, pb = g.op("load ptr, ptr %s" % pa), g.op("load ptr, ptr %s" % pb)
            e = g9_eq_val(g, f.type, pa, pb)
            nxt = g.label("ceqn")
            g.condbr(e, nxt, fail)
            g.place(nxt)
        g.term("ret i1 true")
    if isinstance(d, StructDecl):
        conj(None, d.fields)
    else:
        tl = g.op("load i64, ptr %s" % g.tag_ptr(ty, "%a"))
        tr = g.op("load i64, ptr %s" % g.tag_ptr(ty, "%b"))
        same, dflt = g.label("ceqs"), g.label("ceqt")
        g.condbr(g.op("icmp eq i64 %s, %s" % (tl, tr)), same, fail)
        g.place(same)
        vs = [(vn, t, fs) for vn, t, fs in g9_variants(d) if fs]
        lbls = [g.label("ceqv") for _ in vs]
        g.term("switch i64 %s, label %%%s [ %s ]" % (tl, dflt, " ".join(
            "i64 %d, label %%%s" % (t, l) for (_vn, t, _fs), l in zip(vs, lbls))))
        g.place(dflt)
        g.term("ret i1 true")
        for (vn, _t, fs), l in zip(vs, lbls):
            g.place(l)
            conj(vn, fs)
    g.place(fail)
    g.term("ret i1 false")
    return g.render("define internal i1 @pruv_ceq__%s(ptr %%a, ptr %%b, ptr %%bud)" % d.name)


def g9_helper_chunks(m, tt):
    """clone / drop / node helpers of every extended type (SCC-condensed order), then the contract size / equality
    helpers the requires checks needed (closed over the nominal field types, declaration order)"""
    boxed = tt.g9_boxed
    targets = set()
    for d in tt.order:
        for vn, _t, fs in g9_variants(d):
            for f in fs:
                if fld_label(d, vn, f.name) in boxed:
                    targets.add(f.type.name)
    chunks = []
    for d in tt.order:
        if d.name not in tt.g9_ext:
            continue
        chunks.append(g9_drop_text(m, tt, d))
        chunks.append(g9_clone_text(m, tt, d))
        if d.name in targets:
            chunks.extend(g9_node_texts(m, tt, d))

    def close(seed):
        out, work = set(), sorted(seed)
        while work:
            n = work.pop()
            if n in out or n not in tt.decls:
                continue
            out.add(n)
            for _vn, _t, fs in g9_variants(tt.decls[n]):
                for f in fs:
                    work.extend(codegen.nominals_in(f.type))
        return out
    sizes, eqs = close(m.need_size), close(m.need_eq)
    for d in tt.decls.values():
        if d.name in sizes:
            chunks.append(g9_size_text(m, tt, d))
        if d.name in eqs:
            chunks.append(g9_eq_text(m, tt, d))
    return chunks


class Words:
    """IR text for the external word form of user types (docs/M2_G1_SPEC.md section 5, W(T))"""

    def __init__(s, tt):
        s.tt, s.lines, s.n = tt, [], 0
        s.checks = []       # G3: (valid i1, raw tag word, type name) of every enum ordinal that must be rejected if invalid
        s.stm, s.hn = {}, {}  # caches: storage type text per type, "contains a G3 application" per type

    def S(s, ty):
        """st(ty) (non-contract form), cached per type"""
        r = s.stm.get(ty)
        if r is None:
            if gopt(ty):
                x = s.S(ty.args[0])
                r = "{ i1, %s }" % x if ty.name == "option" else "{ i1, %s, %s }" % (x, s.S(result_error(ty)))
            else:
                r = st(ty)
            s.stm[ty] = r
        return r

    def has_new(s, ty):
        """whether ty is, or contains by value, a G3 application (its words follow the G3 rules)"""
        r = s.hn.get(ty)
        if r is None:
            if is_new_app(ty):
                r = True
            elif not is_user_type(ty):
                r = False
            else:
                d = s.tt.decl(ty)
                fs = d.fields if isinstance(d, StructDecl) else [f for v in d.variants for f in v.fields]
                r = any(s.has_new(f.type) for f in fs)
            s.hn[ty] = r
        return r

    def op(s, text):
        s.n += 1
        r = "%%w.%d" % s.n
        s.lines.append("%s = %s" % (r, text))
        return r

    def gep(s, ptr, off):
        return s.op("getelementptr inbounds i64, ptr %s, i64 %d" % (ptr, off))

    def rd(s, ptr, off):
        return s.op("load i64, ptr %s" % s.gep(ptr, off))

    def wr(s, ptr, off, v):
        g = s.gep(ptr, off)
        s.lines.append("store i64 %s, ptr %s" % (v, g))

    def de(s, ty, ptr, off, act="true"):
        """read a value of type ty from words at ptr[off..]; returns an SSA name of type st(ty).
        Legacy types keep the G1 reading; a G3 application (new Option/Result, generated generic instance) and
        everything below it is read by de_new (masked inactive words, enum ordinal checks).  act is "true" or an i1
        telling whether this legacy value is active: it only matters for a G3 application reached through legacy
        structs/enums, whose ordinal check happens only when its whole path is active (an inactive one reads zero)."""
        n = ty.name
        if is_new_app(ty):
            return s.de_new(ty, ptr, off, act)
        if n == "i64":
            return s.rd(ptr, off)
        if n == "bool":
            return s.op("icmp ne i64 %s, 0" % s.rd(ptr, off))
        if n in ("option", "result"):
            t = s.op("icmp ne i64 %s, 0" % s.rd(ptr, off))
            p = s.rd(ptr, off + 1)
            a = s.op("insertvalue %s undef, i1 %s, 0" % (OPT, t))
            return s.op("insertvalue %s %s, i64 %s, 1" % (OPT, a, p))
        d = s.tt.decl(ty)
        T = st(ty)
        acc = "zeroinitializer"
        if isinstance(d, StructDecl):
            o = off
            for i, f in enumerate(d.fields):
                x = s.de(f.type, ptr, o, act)
                acc = s.op("insertvalue %s %s, %s %s, %d" % (T, acc, s.S(f.type), x, i))
                o += words(s.tt, f.type)
            return acc
        tag = s.rd(ptr, off)
        acc = s.op("insertvalue %s zeroinitializer, i64 %s, 0" % (T, tag))
        o, idx = off + 1, 1
        for k, v in enumerate(d.variants):
            ak = act
            if any(s.has_new(f.type) for f in v.fields):    # only then is the activity of this payload observable
                ak = s.op("icmp eq i64 %s, %d" % (tag, k))
                if act != "true":
                    ak = s.op("and i1 %s, %s" % (act, ak))
            for f in v.fields:
                x = s.de(f.type, ptr, o, ak)
                acc = s.op("insertvalue %s %s, %s %s, %d" % (T, acc, s.S(f.type), x, idx))
                o += words(s.tt, f.type)
                idx += 1
        return acc

    def de_new(s, ty, ptr, off, act):
        """G3 reading of a value of type ty from words at ptr[off..].  act is "true" or an i1 SSA name telling
        whether this value is semantically active: inactive words are read as zero (deterministic storage, ignored
        semantically), bool/tag words normalize by nonzero-to-true, and the ordinal of every nominal enum that is
        active must be < the variant count (recorded in s.checks; the wrapper rejects before requires/body)."""
        n = ty.name
        on = act != "true"
        if n == "i64":
            w = s.rd(ptr, off)
            return s.op("select i1 %s, i64 %s, i64 0" % (act, w)) if on else w
        if n == "bool":
            c = s.op("icmp ne i64 %s, 0" % s.rd(ptr, off))
            return s.op("and i1 %s, %s" % (act, c)) if on else c
        if gopt(ty):
            X = ty.args[0]
            T = s.S(ty)
            t = s.op("icmp ne i64 %s, 0" % s.rd(ptr, off))
            if on:
                t = s.op("and i1 %s, %s" % (act, t))
            x = s.de_new(X, ptr, off + 1, t)
            acc = s.op("insertvalue %s zeroinitializer, i1 %s, 0" % (T, t))
            acc = s.op("insertvalue %s %s, %s %s, 1" % (T, acc, s.S(X), x))
            if n == "result" and result_error(ty) == I64:
                ew = s.rd(ptr, off + 1 + words(s.tt, X))
                ea = s.op("xor i1 %s, true" % t)
                if on:
                    ea = s.op("and i1 %s, %s" % (act, ea))
                e = s.op("select i1 %s, i64 %s, i64 0" % (ea, ew))
                acc = s.op("insertvalue %s %s, i64 %s, 2" % (T, acc, e))
            elif n == "result":     # LG12: an error payload of type E, active when the tag is clear
                E = result_error(ty)
                ea = s.op("xor i1 %s, true" % t)
                if on:
                    ea = s.op("and i1 %s, %s" % (act, ea))
                e = s.de_new(E, ptr, off + 1 + words(s.tt, X), ea)
                acc = s.op("insertvalue %s %s, %s %s, 2" % (T, acc, s.S(E), e))
            return acc
        if n in ("option", "result"):       # legacy Option<i64>/Result<i64> nested in a G3 value
            t = s.op("icmp ne i64 %s, 0" % s.rd(ptr, off))
            if on:
                t = s.op("and i1 %s, %s" % (act, t))
            p = s.rd(ptr, off + 1)
            pa = t if n == "option" else act
            if pa != "true":
                p = s.op("select i1 %s, i64 %s, i64 0" % (pa, p))
            a = s.op("insertvalue %s undef, i1 %s, 0" % (OPT, t))
            return s.op("insertvalue %s %s, i64 %s, 1" % (OPT, a, p))
        d = s.tt.decl(ty)
        T = st(ty)
        acc = "zeroinitializer"
        if isinstance(d, StructDecl):
            o = off
            for i, f in enumerate(d.fields):
                x = s.de_new(f.type, ptr, o, act)
                acc = s.op("insertvalue %s %s, %s %s, %d" % (T, acc, s.S(f.type), x, i))
                o += words(s.tt, f.type)
            return acc
        w = s.rd(ptr, off)
        ok = s.op("icmp ult i64 %s, %d" % (w, len(d.variants)))
        if on:
            ok = s.op("or i1 %s, %s" % (s.op("xor i1 %s, true" % act), ok))
        s.checks.append((ok, w, getattr(d, "disp", ty.name)))   # G4: source-qualified type identity in explicit mode
        tag = s.op("select i1 %s, i64 %s, i64 0" % (act, w)) if on else w
        acc = s.op("insertvalue %s zeroinitializer, i64 %s, 0" % (T, tag))
        o, idx = off + 1, 1
        for k, v in enumerate(d.variants):
            ak = None
            if v.fields:
                ak = s.op("icmp eq i64 %s, %d" % (w, k))
                if on:
                    ak = s.op("and i1 %s, %s" % (act, ak))
            for f in v.fields:
                x = s.de_new(f.type, ptr, o, ak)
                acc = s.op("insertvalue %s %s, %s %s, %d" % (T, acc, s.S(f.type), x, idx))
                o += words(s.tt, f.type)
                idx += 1
        return acc

    def ser_new(s, ty, x, ptr, off, act):
        """G3 writing of the SSA value x (type st(ty)) as words at ptr[off..].  act is "true" or an i1 telling whether
        this value is semantically active.  Every inactive word is written explicitly as zero (a legacy-ABI value
        may carry dirty inactive storage), propagating the activity recursively: an Option payload is active when
        the tag is set, a Result error word when it is not, an enum payload when the tag selects its variant."""
        n = ty.name
        on = act != "true"
        if n == "i64":
            s.wr(ptr, off, s.op("select i1 %s, i64 %s, i64 0" % (act, x)) if on else x)
        elif n == "bool":
            b = s.op("and i1 %s, %s" % (act, x)) if on else x
            s.wr(ptr, off, s.op("zext i1 %s to i64" % b))
        elif gopt(ty):  # tag word, payload words, (Result) error word
            X = ty.args[0]
            T = s.S(ty)
            t = s.op("extractvalue %s %s, 0" % (T, x))
            if on:
                t = s.op("and i1 %s, %s" % (act, t))
            s.wr(ptr, off, s.op("zext i1 %s to i64" % t))
            s.ser_new(X, s.op("extractvalue %s %s, 1" % (T, x)), ptr, off + 1, t)
            if n == "result":   # LG12: the error payload of type E (one i64 word for Result<X>)
                e = s.op("extractvalue %s %s, 2" % (T, x))
                ea = s.op("xor i1 %s, true" % t)
                if on:
                    ea = s.op("and i1 %s, %s" % (act, ea))
                s.ser_new(result_error(ty), e, ptr, off + 1 + words(s.tt, X), ea)
        elif n in ("option", "result"):      # legacy Option<i64>/Result<i64> nested in a G3 value
            t = s.op("extractvalue %s %s, 0" % (OPT, x))
            p = s.op("extractvalue %s %s, 1" % (OPT, x))
            if on:
                t = s.op("and i1 %s, %s" % (act, t))
            pa = t if n == "option" else act
            if pa != "true":
                p = s.op("select i1 %s, i64 %s, i64 0" % (pa, p))
            s.wr(ptr, off, s.op("zext i1 %s to i64" % t))
            s.wr(ptr, off + 1, p)
        else:
            d = s.tt.decl(ty)
            T = st(ty)
            if isinstance(d, StructDecl):
                if not d.fields:
                    s.wr(ptr, off, "0")
                o = off
                for i, f in enumerate(d.fields):
                    s.ser_new(f.type, s.op("extractvalue %s %s, %d" % (T, x, i)), ptr, o, act)
                    o += words(s.tt, f.type)
                return
            tag = s.op("extractvalue %s %s, 0" % (T, x))
            s.wr(ptr, off, s.op("select i1 %s, i64 %s, i64 0" % (act, tag)) if on else tag)
            o, idx = off + 1, 1
            for k, v in enumerate(d.variants):
                ak = None
                if v.fields:
                    ak = s.op("icmp eq i64 %s, %d" % (tag, k))
                    if on:
                        ak = s.op("and i1 %s, %s" % (act, ak))
                for f in v.fields:
                    s.ser_new(f.type, s.op("extractvalue %s %s, %d" % (T, x, idx)), ptr, o, ak)
                    o += words(s.tt, f.type)
                    idx += 1

    def ser(s, ty, x, ptr, off, act="true"):
        """write the SSA value x (type st(ty)) as words at ptr[off..].  A purely legacy value is written as is (the
        legacy ABI output stays byte-identical); a G3 application and everything nested in it goes through ser_new,
        also when it hangs below a legacy struct/enum (act: whether that legacy value is active)."""
        n = ty.name
        if is_new_app(ty):
            s.ser_new(ty, x, ptr, off, act)
        elif n == "i64":
            s.wr(ptr, off, x)
        elif n == "bool":
            s.wr(ptr, off, s.op("zext i1 %s to i64" % x))
        elif n in ("option", "result"):
            t = s.op("extractvalue %s %s, 0" % (OPT, x))
            p = s.op("extractvalue %s %s, 1" % (OPT, x))
            if n == "option":           # None payload is serialized as 0
                p = s.op("select i1 %s, i64 %s, i64 0" % (t, p))
            s.wr(ptr, off, s.op("zext i1 %s to i64" % t))
            s.wr(ptr, off + 1, p)
        else:
            d = s.tt.decl(ty)
            T = st(ty)
            if isinstance(d, StructDecl):
                if not d.fields:
                    s.wr(ptr, off, "0")
                o = off
                for i, f in enumerate(d.fields):
                    s.ser(f.type, s.op("extractvalue %s %s, %d" % (T, x, i)), ptr, o, act)
                    o += words(s.tt, f.type)
                return
            # legacy words are written as they are; only a G3 application below has its inactivity made explicit
            tag = s.op("extractvalue %s %s, 0" % (T, x))
            s.wr(ptr, off, tag)
            o, idx = off + 1, 1
            for k, v in enumerate(d.variants):
                ak = act
                if any(s.has_new(f.type) for f in v.fields):
                    ak = s.op("icmp eq i64 %s, %d" % (tag, k))
                    if act != "true":
                        ak = s.op("and i1 %s, %s" % (act, ak))
                for f in v.fields:
                    s.ser(f.type, s.op("extractvalue %s %s, %d" % (T, x, idx)), ptr, o, ak)
                    o += words(s.tt, f.type)
                    idx += 1


def overlap_lines(f, tt, m):
    """mutable host entries: the first lines of a wrapper with an inout parameter reject (requires violation, also under
    --trust-callers) an inout region that overlaps another pointer-carrying argument or the result storage.
    Pointer-carrying list: array/str params and aggregate params in declaration order, then `%out`; a pair is checked
    when at least one of its two members is inout. A lone inout region (the only pointer-carrying item) is still
    validated: it is checked against an empty sentinel region (`ptr null, i64 0`)."""
    if not has_inout(f):
        return []
    items = []   # (ptr, len, elem bytes, is_inout)
    for p in f.params:
        q = mangle(p.name)
        if is_agg(p.type):
            items.append(("%%a.%s" % q, str(words(tt, p.type) * 8), 1, p.mode == "inout"))
        elif p.type in (I64, BOOL) or not is_arr(p.type):
            continue   # scalars and Option/Result (two registers) carry no pointer
        else:
            e = 1 if p.type.name in ("array_u8", "array_bool", "str") else 8
            items.append(("%%a.%s.a" % q, "%%a.%s.n" % q, e, p.mode == "inout"))
    if is_arr(f.ret):
        items.append(("%out", "16", 1, False))
    elif is_agg(f.ret):
        items.append(("%out", str(words(tt, f.ret) * 8), 1, False))
    if len(items) == 1:
        # a lone inout region has no partner, but @pruv_overlap also rejects a negative length / size overflow of one
        # side: check it against the empty sentinel region (an empty region never overlaps)
        items.append(("null", "0", 1, False))
    pairs = [(a, b) for i, a in enumerate(items) for b in items[i + 1:] if a[3] or b[3]]
    if not pairs:
        return []
    m.overlap = True
    out = []
    for k, (a, b) in enumerate(pairs):
        out.append("  %%ov.%d = call i1 @pruv_overlap(ptr %s, i64 %s, i64 %d, ptr %s, i64 %s, i64 %d)"
                   % (k, a[0], a[1], a[2], b[0], b[1], b[2]))
    flag = "%ov.0"
    for k in range(1, len(pairs)):
        out.append("  %%ov.or.%d = or i1 %s, %%ov.%d" % (k, flag, k))
        flag = "%%ov.or.%d" % k
    fn = m.string(getattr(f, "disp", f.name))
    msg = m.string("inout argument overlaps another argument")
    out += ["  br i1 %s, label %%ovl.bad, label %%ovl.ok" % flag,
            "ovl.bad:",
            "  call void @pruv_panic_requires(ptr %s, ptr %s)" % (fn, msg),
            "  unreachable",
            "ovl.ok:"]
    return out


LEN_MAX = 1 << 40  # the Pruv array length bound (SPEC section 7 item 6), assumed by the verifier for every array parameter


def len_lines(f, m):
    """T0: every array/str parameter of a host ABI wrapper (both pruv_abi_* and __typed) is rejected unless its length is
    in 0..=2^40, before the representation checks (which read the buffer), requires and the body. One unsigned compare
    covers a negative length. With two or more such parameters the fast path is one compare on the OR of the lengths
    (a negative length sets the top bit); only when it fails are the lengths checked one by one, in parameter order,
    and a slow path that finds none above 2^40 (one is exactly 2^40) goes on. The message names the parameter kind:
    `str length` keeps the old negative-length text."""
    ps = [p for p in f.params if is_arr(p.type)]
    if not ps:
        return []
    out = []
    if len(ps) > 1:
        acc = "%%a.%s.n" % mangle(ps[0].name)
        for k, p in enumerate(ps[1:]):
            out.append("  %%ln.or.%d = or i64 %s, %%a.%s.n" % (k, acc, mangle(p.name)))
            acc = "%%ln.or.%d" % k
        out += ["  %%ln.any = icmp ugt i64 %s, %d" % (acc, LEN_MAX),
                "  br i1 %ln.any, label %len.slow, label %len.ok",
                "len.slow:"]
    # names use the parameter's position, not its name: a parameter called `any` or `or` must not collide
    for i, p in enumerate(ps):
        q = mangle(p.name)
        msg = m.string("str length" if p.type.name == "str" else "array length")
        m.uses_repr = True
        out += ["  %%ln.p%d = icmp ugt i64 %%a.%s.n, %d" % (i, q, LEN_MAX),
                "  br i1 %%ln.p%d, label %%len.bad.%d, label %%len.ok.%d" % (i, i, i),
                "len.bad.%d:" % i,
                "  call void @pruv_panic_repr(i64 %%a.%s.n, ptr %s)" % (q, msg),
                "  unreachable",
                "len.ok.%d:" % i]
    if len(ps) > 1:
        out += ["  br label %len.ok", "len.ok:"]
    return out


def abi_of(f, tt, m=None, target="entry"):
    """target: which internal function the wrapper calls, `entry` (checked, legacy) or `body` (G4 explicit mode when the
    host boundary needs no requires check). The representation checks below run in every mode."""
    # defensive: an extended signature never has a wrapper; an inout function only as an explicit-mode host entry
    if has_ext_sig(f, tt) or (has_inout(f) and not (m is not None and m.explicit and f.host_entry)):
        raise Exception("llvmgen: %s has no host ABI wrapper (inout or extended signature)" % f.name)
    n = mangle(f.name)
    ext, pre, args = [], [], []
    w = Words(tt)
    for p in f.params:
        q = mangle(p.name)
        if is_agg(p.type):
            ext.append("ptr %%a.%s" % q)
            args.append("%s %s" % (lty(p.type), w.de(p.type, "%%a.%s" % q, 0)))
            pre += w.lines
            w.lines = []
        elif p.type == I64:
            ext.append("i64 %%a.%s" % q)
            args.append("i64 %%a.%s" % q)
        elif p.type == BOOL:
            ext.append("i64 %%a.%s" % q)
            pre.append("%%c.%s = icmp ne i64 %%a.%s, 0" % (q, q))
            args.append("i1 %%c.%s" % q)
        elif is_arr(p.type):
            ext += ["ptr %%a.%s.a" % q, "i64 %%a.%s.n" % q]
            if p.type.name == "array_bool":  # G2 A5: representation check before entry
                pre.append("call void @pruv_check_bool(ptr %%a.%s.a, i64 %%a.%s.n)" % (q, q))
            elif p.type.name == "str":  # G2 B2: UTF-8 check before requires/body
                pre.append("call void @pruv_check_utf8(ptr %%a.%s.a, i64 %%a.%s.n)" % (q, q))
            args += ["ptr %%a.%s.a" % q, "i64 %%a.%s.n" % q]
        else:
            ext += ["i64 %%a.%s.t" % q, "i64 %%a.%s.v" % q]
            pre.append("%%c.%s.t = icmp ne i64 %%a.%s.t, 0" % (q, q))
            pre.append("%%c.%s.g1 = insertvalue %s undef, i1 %%c.%s.t, 0" % (q, OPT, q))
            pre.append("%%c.%s.g = insertvalue %s %%c.%s.g1, i64 %%a.%s.v, 1" % (q, OPT, q, q))
            args.append("%s %%c.%s.g" % (OPT, q))
    rt = f.ret
    entry = "@%s__pruv_%s" % (n, target)
    body = overlap_lines(f, tt, m) + len_lines(f, m) + ["  " + x for x in pre]
    for k, (okv, wv, tname) in enumerate(w.checks):
        # G3: an invalid nominal enum ordinal is rejected here, before the entry (requires) and the body run
        msg = m.string("enum " + tname)
        m.uses_repr = True
        body.append("  br i1 %s, label %%enum.ok%d, label %%enum.bad%d" % (okv, k, k))
        body.append("enum.bad%d:" % k)
        body.append("  call void @pruv_panic_repr(i64 %s, ptr %s)" % (wv, msg))
        body.append("  unreachable")
        body.append("enum.ok%d:" % k)
    if is_arr(rt):
        ext.append("ptr %out")
        body.append("  %%r = call %s %s(%s)" % (lty(rt), entry, ", ".join(args)))
        body.append("  %%x0r = extractvalue %s %%r, 0" % lty(rt))
        body.append("  %%x1 = extractvalue %s %%r, 1" % lty(rt))
        body.append("  %%x2 = extractvalue %s %%r, 2" % lty(rt))
        # P3: the host frees with the exact length, so the buffer is normalized to cap == len first (only a moved
        # truncate has cap != len; the cap is a runtime value here, so pruv_shrink_v2 runs only when cap != len)
        if rt.name in ("array_u8", "array_bool", "str"):
            sz = 1
        else:
            sz = elem_size(m.storage.layout_of(f)) if m is not None else 8
        if m is not None:
            m.shrink = True
        body.append("  %o0 = getelementptr inbounds { ptr, i64 }, ptr %out, i32 0, i32 0")
        body.append("  %o1 = getelementptr inbounds { ptr, i64 }, ptr %out, i32 0, i32 1")
        body.append("  store i64 %x1, ptr %o1")
        body.append("  %x3 = icmp eq i64 %x2, %x1")
        body.append("  br i1 %x3, label %shr.same, label %shr.do")
        body.append("shr.do:")
        body.append("  %%x0 = call ptr @pruv_shrink_v2(ptr %%x0r, i64 %%x2, i64 %%x1, i64 %d, i64 %d)" % (sz, sz))
        body.append("  store ptr %x0, ptr %o0")
        body.append("  ret void")
        body.append("shr.same:")
        body.append("  store ptr %x0r, ptr %o0")
        body.append("  ret void")
        ret = "void"
    elif rt in (OPTION, RESULT):
        # returned by value as { i64, i64 } (two registers on macOS arm64 / SysV x86-64)
        body.append("  %%r = call %s %s(%s)" % (lty(rt), entry, ", ".join(args)))
        body.append("  %%x0 = extractvalue %s %%r, 0" % lty(rt))
        body.append("  %%x1 = extractvalue %s %%r, 1" % lty(rt))
        body.append("  %x0z = zext i1 %x0 to i64")
        body.append("  %t0 = insertvalue { i64, i64 } undef, i64 %x0z, 0")
        body.append("  %t1 = insertvalue { i64, i64 } %t0, i64 %x1, 1")
        body.append("  ret { i64, i64 } %t1")
        ret = "{ i64, i64 }"
    elif is_agg(rt):
        ext.append("ptr %out")
        body.append("  %%r = call %s %s(%s)" % (lty(rt), entry, ", ".join(args)))
        w.ser(rt, "%r", "%out", 0)
        body += ["  " + x for x in w.lines]
        body.append("  ret void")
        ret = "void"
    elif rt == UNIT:
        body.append("  call void %s(%s)" % (entry, ", ".join(args)))
        body.append("  ret void")
        ret = "void"
    elif rt == BOOL:
        body.append("  %%r = call i1 %s(%s)" % (entry, ", ".join(args)))
        body.append("  %rz = zext i1 %r to i64")
        body.append("  ret i64 %rz")
        ret = "i64"
    else:
        body.append("  %%r = call i64 %s(%s)" % (entry, ", ".join(args)))
        body.append("  ret i64 %r")
        ret = "i64"
    sp = m.sym_pre if m is not None else ""
    out = "define %s @%spruv_abi_%s(%s) {\nentry:\n%s\n}\n" % (ret, sp, n, ", ".join(ext), "\n".join(body))
    if any(p.type.name in ("array_bool", "str") for p in f.params):
        # G2 A5/B2: typed Rust bindings pass &[bool] / &str (valid by type) to this entry without the check
        tb = [x for x in body if not (x.startswith("  call void @pruv_check_bool(")
                                      or x.startswith("  call void @pruv_check_utf8("))]
        out += "\ndefine %s @%spruv_abi_%s__typed(%s) {\nentry:\n%s\n}\n" % (ret, sp, n, ", ".join(ext), "\n".join(tb))
    return out


# ---------- helper functions for requires ----------
def cdivrem(m, is_div):
    nm, sym = ("pruv_cdiv", "/") if is_div else ("pruv_crem", "%")
    z = m.string("division by zero in " + sym)
    o = m.string("i128 overflow in " + sym)
    res = "%qq" if is_div else "%rr"
    return """\
define internal i128 @%s(i128 %%a, i128 %%b) {
entry:
  %%z = icmp eq i128 %%b, 0
  br i1 %%z, label %%dz, label %%c1
dz:
  call void @pruv_panic_undefined(ptr %s)
  unreachable
c1:
  %%am = icmp eq i128 %%a, %d
  %%bm = icmp eq i128 %%b, -1
  %%ov = and i1 %%am, %%bm
  br i1 %%ov, label %%of, label %%c2
of:
  call void @pruv_panic_undefined(ptr %s)
  unreachable
c2:
  %%q = sdiv i128 %%a, %%b
  %%r = srem i128 %%a, %%b
  %%neg = icmp slt i128 %%r, 0
  %%bp = icmp sgt i128 %%b, 0
  %%ra = add i128 %%r, %%b
  %%rs = sub i128 %%r, %%b
  %%r2 = select i1 %%bp, i128 %%ra, i128 %%rs
  %%rr = select i1 %%neg, i128 %%r2, i128 %%r
  %%qd = select i1 %%bp, i128 -1, i128 1
  %%q2 = add i128 %%q, %%qd
  %%qq = select i1 %%neg, i128 %%q2, i128 %%q
  ret i128 %s
}
""" % (nm, z, SIGNED_MIN128, o, res)


# ---------- Rust bindings ----------
def typed_abi_name(f):
    """G2 A5: the bindings call pruv_abi_<f>__typed (no [bool] byte check) when f has a [bool] parameter"""
    return f.name + ("__typed" if any(p.type.name in ("array_bool", "str") for p in f.params) else "")  # G2 B2


def rust_ty(t, param):
    if gopt(t):  # G3: ::std::option::Option<R(X)> / ::std::result::Result<R(X), i64>
        return codegen.rty(t)
    if is_user_type(t):
        return t.name
    if t.name == "array_u8":
        return "&[u8]" if param else "::std::vec::Vec<u8>"
    if t.name == "array_bool":  # G2 A5
        return "&[bool]" if param else "::std::vec::Vec<bool>"
    if t.name == "str":  # G2 B2
        return "&str" if param else "::std::string::String"
    return {"i64": "i64", "bool": "bool", "unit": "()", "option": "::std::option::Option<i64>",
            "result": "::std::result::Result<i64, i64>"}.get(t.name) or (
        "&[i64]" if param else "::std::vec::Vec<i64>")


ALLOW = "#[allow(dead_code, non_snake_case, unused)]"


def rs_ser(tt, ty, expr, arr, off, ctr):
    """Rust statements writing `expr` (a Copy value of type ty) into the word array `arr` at off"""
    n = ty.name
    if n == "i64":
        return ["%s[%d] = %s;" % (arr, off, expr)]
    if n == "bool":
        return ["%s[%d] = (%s) as i64;" % (arr, off, expr)]
    if gopt(ty):  # G3: the word array starts zeroed, so only the active words are written
        ctr[0] += 1
        v = "pruv__b%d" % ctr[0]
        inner = " ".join(rs_ser(tt, ty.args[0], v, arr, off + 1, ctr))
        if n == "option":
            return ["match %s { Some(%s) => { %s[%d] = 1; %s } None => { %s[%d] = 0; } }"
                    % (expr, v, arr, off, inner, arr, off)]
        err = " ".join(rs_ser(tt, result_error(ty), v, arr, off + 1 + words(tt, ty.args[0]), ctr))  # LG12: E words
        return ["match %s { Ok(%s) => { %s[%d] = 1; %s } Err(%s) => { %s[%d] = 0; %s } }"
                % (expr, v, arr, off, inner, v, arr, off, err)]
    if n in ("option", "result"):
        ctr[0] += 1
        v = "pruv__b%d" % ctr[0]
        a, b = ("Some", "None") if n == "option" else ("Ok", "Err")
        if n == "option":
            return ["match %s { Some(%s) => { %s[%d] = 1; %s[%d] = %s; } None => { %s[%d] = 0; %s[%d] = 0; } }"
                    % (expr, v, arr, off, arr, off + 1, v, arr, off, arr, off + 1)]
        return ["match %s { Ok(%s) => { %s[%d] = 1; %s[%d] = %s; } Err(%s) => { %s[%d] = 0; %s[%d] = %s; } }"
                % (expr, v, arr, off, arr, off + 1, v, v, arr, off, arr, off + 1, v)]
    d = tt.decl(ty)
    if isinstance(d, StructDecl):
        out, o = [], off
        for f in d.fields:
            out += rs_ser(tt, f.type, "%s.%s" % (expr, f.name), arr, o, ctr)
            o += words(tt, f.type)
        return out
    arms, o = [], off + 1
    for k, v in enumerate(d.variants):
        names, stmts = [], ["%s[%d] = %d;" % (arr, off, k)]
        for f in v.fields:
            ctr[0] += 1
            nm = "pruv__b%d" % ctr[0]
            names.append(nm)
            stmts += rs_ser(tt, f.type, nm, arr, o, ctr)
            o += words(tt, f.type)
        pat = "%s::%s(%s)" % (d.name, v.name, ", ".join(names)) if names else "%s::%s" % (d.name, v.name)
        arms.append("%s => { %s }" % (pat, " ".join(stmts)))
    return ["match %s { %s }" % (expr, " ".join(arms))]


def rs_de(tt, ty, arr, off):
    """Rust expression rebuilding a value of type ty from the word array `arr` at off (safe constructors only)"""
    n = ty.name
    if n == "i64":
        return "%s[%d]" % (arr, off)
    if n == "bool":
        return "(%s[%d] != 0)" % (arr, off)
    if gopt(ty):  # G3
        x = rs_de(tt, ty.args[0], arr, off + 1)
        if n == "option":
            return "(if %s[%d] != 0 { Some(%s) } else { None })" % (arr, off, x)
        e = rs_de(tt, result_error(ty), arr, off + 1 + words(tt, ty.args[0]))     # LG12: E words
        return "(if %s[%d] != 0 { Ok(%s) } else { Err(%s) })" % (arr, off, x, e)
    if n == "option":
        return "(if %s[%d] != 0 { Some(%s[%d]) } else { None })" % (arr, off, arr, off + 1)
    if n == "result":
        return "(if %s[%d] != 0 { Ok(%s[%d]) } else { Err(%s[%d]) })" % (arr, off, arr, off + 1, arr, off + 1)
    d = tt.decl(ty)
    if isinstance(d, StructDecl):
        parts, o = [], off
        for f in d.fields:
            parts.append("%s: %s" % (f.name, rs_de(tt, f.type, arr, o)))
            o += words(tt, f.type)
        return "%s { %s }" % (d.name, ", ".join(parts))
    arms, o = [], off + 1
    for k, v in enumerate(d.variants):
        args = []
        for f in v.fields:
            args.append(rs_de(tt, f.type, arr, o))
            o += words(tt, f.type)
        arms.append("%d => %s::%s%s," % (k, d.name, v.name, "(%s)" % ", ".join(args) if args else ""))
    return "(match %s[%d] { %s _ => ::std::unreachable!() })" % (arr, off, " ".join(arms))


def is_host_abi_fn(f, tt, explicit):
    """a function that gets `pruv_abi_*` wrappers: legacy = every ordinary function that is not a specialization or
    collection/extended-bearing; G4 explicit mode = additionally only the host entries (a child-public function is
    never exported just because Pruv callers can name it)"""
    return exports_abi(f, tt, explicit)


def entry_type_closure(prog, tt):
    """G4 A3: names of the nominal types reachable from the host entry signatures (parameters and results), following
    Option/Result arguments and struct fields / enum payloads, with a visited set (cycles)"""
    seen = set()
    work = []
    for f in prog.functions:
        if is_host_abi_fn(f, tt, True):
            work += [p.type for p in f.params] + [f.ret]
    while work:
        t = work.pop()
        work += list(t.args or [])
        if is_user_type(t) and t.name not in seen:
            seen.add(t.name)
            d = tt.decls.get(t.name)
            if d is not None:
                fields = d.fields if isinstance(d, StructDecl) else [x for v in d.variants for x in v.fields]
                work += [x.type for x in fields]
    return seen


C_HEAD = """\
// P10: manifest raw_entries = "c": pruv_c is the C adapter. One opaque handle owns the resident state in a
// pruv_raw::PruvStore; every extern "C" function catches a Rust panic (no unwind crosses into C) and returns a status.
// A poisoned handle refuses every call but is still freed. Only the LLVM bindings carry this module.
// P10H: every global symbol carries the program prefix <pkg>__ so two Pruv programs link into one C binary.
#[allow(dead_code, unexpected_cfgs)]
pub mod pruv_c {
    //! Caller contract of every `pub unsafe extern "C" fn` below (the adapter cannot check what it cannot see):
    //! - the handle comes from @P@pruv_handle_new and has not been freed;
    //! - no concurrent or reentrant use of one handle;
    //! - each (pointer, length) pair with length > 0 points to `length` initialized elements inside one live
    //!   allocation, readable (writable for an inout parameter) and not accessed by anyone else for the duration
    //!   of the call;
    //! - ret points to a writable i64.
    //!
    //! The adapter checks null, sign, alignment, size (every array, new state and rebuilt state has at most 2^40
    //! elements, the Pruv array bound), address overflow and overlap among the call's buffers and the handle, but it
    //! cannot check allocation size or provenance.
    /// Statuses of every @P@pruv_h_* / @P@pruv_hr_* / @P@pruv_hb_* call: 0 ok, 1 poisoned (refused before any argument
    /// is read, nothing ran), 2 bad arguments (refused, nothing ran), 3 a Rust panic unwound to the adapter (if it
    /// happened inside the store it poisoned the handle). A batch (`hb`) answers 1 and 2 for the whole call before any
    /// item runs, except a bad item, which is answered 2 after the items before it ran. Checked failures inside
    /// LLVM-compiled code (range, arithmetic, requires) and G6 host-callback failures end the process (exit 101);
    /// they never return a status.
    pub const PRUV_OK: i32 = 0;
    pub const PRUV_POISONED: i32 = 1;
    pub const PRUV_BAD_ARGS: i32 = 2;
    pub const PRUV_PANICKED: i32 = 3;
    /// Opaque to C (one pointer). Owns the resident state; the C side never sees the state buffer.
    pub struct PruvHandle {
        store: super::pruv_raw::PruvStore<::std::vec::Vec<i64>>,
    }
    // The Pruv array length bound (SPEC section 7 item 6, assumed by the verifier for every array parameter):
    // 0 <= n <= min(2^40, isize::MAX / size_of::<T>()). Checked once where a C buffer or a new or rebuilt state
    // enters proved code; never inside the entries.
    #[inline(always)]
    fn len_ok<T>(n: i64) -> bool {
        n >= 0 && n as u64 <= ((isize::MAX as u64) / (::std::mem::size_of::<T>() as u64)).min(1u64 << 40)
    }
    // A C (pointer, length) pair as a slice: None for a length outside len_ok, or for a null or misaligned
    // pointer with a nonzero length, or whose end address overflows. Length 0 never reads the pointer.
    #[inline(always)]
    unsafe fn ro<'a, T>(p: *const T, n: i64) -> Option<&'a [T]> {
        if n == 0 {
            return Some(&[]);
        }
        if !len_ok::<T>(n)
            || (p as usize).checked_add(n as usize * ::std::mem::size_of::<T>()).is_none() || p.is_null()
            || (p as usize) % ::std::mem::align_of::<T>() != 0 {
            return None;
        }
        Some(unsafe { ::std::slice::from_raw_parts(p, n as usize) })
    }
    #[inline(always)]
    unsafe fn mt<'a, T>(p: *mut T, n: i64) -> Option<&'a mut [T]> {
        if n == 0 {
            return Some(&mut []);
        }
        if !len_ok::<T>(n)
            || (p as usize).checked_add(n as usize * ::std::mem::size_of::<T>()).is_none() || p.is_null()
            || (p as usize) % ::std::mem::align_of::<T>() != 0 {
            return None;
        }
        Some(unsafe { ::std::slice::from_raw_parts_mut(p, n as usize) })
    }
    // Byte range of a (pointer, length) pair; empty for a length <= 0.
    #[inline(always)]
    fn span<T>(p: *const T, n: i64) -> (usize, usize) {
        let s = p as usize;
        (s, s.saturating_add((n.max(0) as usize).saturating_mul(::std::mem::size_of::<T>())))
    }
    // A C table of n elements, read by index with raw reads: true for n == 0, else n is positive and not oversized
    // and the pointer is non-null, aligned and its end address does not overflow.
    #[inline(always)]
    fn tab<T>(p: *const T, n: i64) -> bool {
        n == 0 || (n > 0 && n as u64 <= (isize::MAX as u64) / (::std::mem::size_of::<T>() as u64)
            && (p as usize).checked_add(n as usize * ::std::mem::size_of::<T>()).is_some() && !p.is_null()
            && (p as usize) % ::std::mem::align_of::<T>() == 0)
    }
    // Empty ranges never overlap.
    #[inline(always)]
    fn apart(a: (usize, usize), b: (usize, usize)) -> bool {
        !(a.0 < a.1 && b.0 < b.1 && a.0 < b.1 && b.0 < a.1)
    }
    #[inline(always)]
    fn bools(s: &[u8]) -> Option<&[bool]> {
        if s.iter().all(|b| *b <= 1) { Some(unsafe { &*(s as *const [u8] as *const [bool]) }) } else { None }
    }
    #[inline(always)]
    fn bools_mut(s: &mut [u8]) -> Option<&mut [bool]> {
        if s.iter().all(|b| *b <= 1) { Some(unsafe { &mut *(s as *mut [u8] as *mut [bool]) }) } else { None }
    }
    // Runs one C call; a caught panic becomes PRUV_PANICKED (its payload is leaked, never dropped across C).
    #[inline(always)]
    fn guard(f: impl FnOnce() -> i32) -> i32 {
        match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(f)) {
            Ok(s) => s,
            Err(e) => {
                ::std::mem::forget(e);
                PRUV_PANICKED
            }
        }
    }
    /// A new handle whose state is `len` zero words (call the program's init entry next); null for a length
    /// outside 0..=2^40. Free it with @P@pruv_handle_free.
    #[no_mangle]
    pub extern "C" fn @P@pruv_handle_new(len: i64) -> *mut PruvHandle {
        if !len_ok::<i64>(len) {
            return ::std::ptr::null_mut();
        }
        match ::std::panic::catch_unwind(|| {
            ::std::boxed::Box::into_raw(::std::boxed::Box::new(PruvHandle {
                store: super::pruv_raw::PruvStore::new(::std::vec![0i64; len as usize]),
            }))
        }) {
            Ok(p) => p,
            Err(e) => {
                ::std::mem::forget(e);
                ::std::ptr::null_mut()
            }
        }
    }
    /// Frees a handle and its state, poisoned or not; null is ignored. The handle must come from @P@pruv_handle_new and
    /// is never used again: without a registry a double free or a later call cannot be detected.
    /// # Safety
    /// The pruv_c caller contract (module docs) holds.
    #[no_mangle]
    pub unsafe extern "C" fn @P@pruv_handle_free(h: *mut PruvHandle) {
        if h.is_null() {
            return;
        }
        let _ = guard(|| {
            drop(unsafe { ::std::boxed::Box::from_raw(h) });
            PRUV_OK
        });
    }
    /// 1 if a panic poisoned the handle, 0 if not, PRUV_BAD_ARGS for null.
    /// # Safety
    /// The pruv_c caller contract (module docs) holds.
    #[no_mangle]
    pub unsafe extern "C" fn @P@pruv_handle_poisoned(h: *const PruvHandle) -> i32 {
        match unsafe { h.as_ref() } {
            Some(h) => h.store.is_poisoned() as i32,
            None => PRUV_BAD_ARGS,
        }
    }
    /// Probe builds only (rustc --cfg pruv_c_probe): panics inside the store with a payload whose Drop itself
    /// panics, as a failing call would; guard must never drop the payload.
    /// # Safety
    /// The pruv_c caller contract (module docs) holds.
    #[cfg(pruv_c_probe)]
    #[no_mangle]
    pub unsafe extern "C" fn @P@pruv_handle_probe_panic(h: *mut PruvHandle) -> i32 {
        struct Bomb;
        impl Drop for Bomb {
            fn drop(&mut self) {
                panic!("pruv_c probe payload dropped")
            }
        }
        guard(|| {
            let hd = match unsafe { h.as_mut() } {
                Some(h) => h,
                None => return PRUV_BAD_ARGS,
            };
            match hd.store.with(|_| -> () { ::std::panic::panic_any(Bomb) }) {
                Ok(()) => PRUV_OK,
                Err(_) => PRUV_POISONED,
            }
        })
    }
    /// Probe builds only: the length predicate len_ok for `elem` = 1 (u8 elements) or 8 (i64 elements); 1 if accepted.
    #[cfg(pruv_c_probe)]
    #[no_mangle]
    pub extern "C" fn @P@pruv_handle_probe_len(n: i64, elem: i64) -> i32 {
        (if elem == 1 { len_ok::<u8>(n) } else { len_ok::<i64>(n) }) as i32
    }"""

_C_ARRAY_KINDS = ("ro_i64", "ro_u8", "ro_bool", "str", "mt_i64", "mt_u8", "mt_bool")
_C_PTR = {"ro_i64": "*const i64", "ro_u8": "*const u8", "ro_bool": "*const u8", "str": "*const u8",
          "mt_i64": "*mut i64", "mt_u8": "*mut u8", "mt_bool": "*mut u8"}
_C_CONV = {"ro_i64": "ro(%s, %s_len)", "ro_u8": "ro(%s, %s_len)",
           "str": "ro(%s, %s_len).and_then(|b| ::std::str::from_utf8(b).ok())",
           "ro_bool": "ro(%s, %s_len).and_then(bools)", "mt_i64": "mt(%s, %s_len)", "mt_u8": "mt(%s, %s_len)",
           "mt_bool": "mt(%s, %s_len).and_then(bools_mut)"}
_C_KIND_DOC = {"i64": "i64", "bool": "bool, 0/1 as i64", "ro_i64": "[i64] pointer, length",
               "ro_u8": "[u8] pointer, length", "ro_bool": "[bool] pointer, length; bytes 0/1",
               "str": "str pointer, length; UTF-8", "mt_i64": "inout [i64] pointer, length",
               "mt_u8": "inout [u8] pointer, length", "mt_bool": "inout [bool] pointer, length; bytes 0/1"}


def c_len_first(params):
    """T0: when a UTF-8 / bool scan (str, [bool]) comes before another array parameter, the condition that every
    slice length is in len_ok, checked before any conversion so no buffer is read when a later length is oversized
    (SPEC section 7, C adapter); None when no scan precedes another array (the conversions already check in order)."""
    arrs = [(x, k) for x, k in params if k in _C_ARRAY_KINDS]
    scans = [i for i, (_, k) in enumerate(arrs) if k in ("str", "ro_bool", "mt_bool")]
    if not scans or scans[0] == len(arrs) - 1:
        return None
    return " && ".join("len_ok::<%s>(%s_len)" % ("i64" if k.endswith("i64") else "u8", x) for x, k in arrs)


def c_param_kind(p):
    """P10: the C kind of one parameter (i64, bool, ro_i64, ro_u8, ro_bool, str, mt_i64, mt_u8, mt_bool) or None"""
    inout = p.mode == "inout"
    if p.type == ARRAY:
        return "mt_i64" if inout else "ro_i64"
    if p.type.name in ("array_u8", "array_bool"):
        k = p.type.name[6:]
        return ("mt_" if inout else "ro_") + k
    if inout:
        return None
    if p.type.name == "str":
        return "str"
    if p.type == I64:
        return "i64"
    if p.type == BOOL:
        return "bool"
    return None


def c_entries(pubs):
    """P10: the entries of `pubs` that get a pruv_h_ function: (name, state mode "mt"/"ro", [(param, kind)], ret kind)"""
    out = []
    for f in pubs:
        if not f.params or f.params[0].type != ARRAY:
            continue
        kinds = [c_param_kind(p) for p in f.params[1:]]
        if any(k is None for k in kinds):
            continue
        rk = "i64" if f.ret == I64 else ("bool" if f.ret == BOOL else ("unit" if f.ret == UNIT else None))
        if rk is None:
            continue
        out.append((f.name, "mt" if f.params[0].mode == "inout" else "ro",
                    [(p.name, k) for p, k in zip(f.params[1:], kinds)], rk))
    return out


def c_module(entries, pkg):
    """P10: the pruv_c module text (no trailing newline). entries: (name, state mode "mt"/"ro", [(param name, kind)],
    ret kind "i64"/"bool"/"unit"), each already eligible (c_entries). pkg: the program package name; P10H: every
    global symbol is prefixed `<pkg>__` (the @P@ placeholder of the text below)."""
    if pkg is None:
        raise RuntimeError("internal: c_module: raw_entries \"c\" without a program package name")
    I12, I16, I20 = " " * 12, " " * 16, " " * 20
    parts = [C_HEAD]
    for name, smode, uparams, rk in entries:
        # no user parameter name is a Rust identifier below: the i-th parameter after the state is a{i}
        params = [("a%d" % i, k) for i, (_, k) in enumerate(uparams)]
        sig = ["h: *mut PruvHandle"]
        for x, k in params:
            if k == "i64" or k == "bool":
                sig.append("%s: i64" % x)
            else:
                sig += ["%s: %s" % (x, _C_PTR[k]), "%s_len: i64" % x]
        if rk != "unit":
            sig.append("ret: *mut i64")
        doc = "    /// C entry for `%s`" % name
        if uparams:
            doc += ": " + ", ".join("a%d = `%s` (%s)" % (i, un, _C_KIND_DOC[k]) for i, (un, k) in enumerate(uparams))
        doc += "; the handle's state is its first argument."
        if rk != "unit":
            doc += " On PRUV_OK the result is in *ret."
        L = [doc, "    /// # Safety", "    /// The pruv_c caller contract (module docs) holds.", "    #[no_mangle]",
             '    pub unsafe extern "C" fn @P@pruv_h_%s(%s) -> i32 {' % (name, ", ".join(sig)),
             "        guard(|| unsafe {",
             I12 + "let hd = match h.as_mut() {", I16 + "Some(h) => h,", I16 + "None => return PRUV_BAD_ARGS,",
             I12 + "};",
             I12 + "if hd.store.is_poisoned() {", I16 + "return PRUV_POISONED;", I12 + "}"]
        if rk != "unit":
            L += [I12 + "if ret.is_null() || (ret as usize) % ::std::mem::align_of::<i64>() != 0 || (ret as usize).checked_add(::std::mem::size_of::<i64>()).is_none() {",
                  I16 + "return PRUV_BAD_ARGS;", I12 + "}"]
        for x, k in params:
            if k == "bool":
                L += [I12 + "if %s != 0 && %s != 1 {" % (x, x), I16 + "return PRUV_BAD_ARGS;", I12 + "}"]
        # Each byte range is computed once (P10.3: recomputing it per pair cost ~1 ns a call).
        b = [("sp_%s" % x, "span(%s, %s_len)" % (x, x), k.startswith("mt_")) for x, k in params if k in _C_ARRAY_KINDS]
        if rk != "unit":
            b.append(("sp_ret", "span(ret, 1)", True))
        b.append(("sp_h", "span(h as *const PruvHandle, 1)", True))
        pairs = ["apart(%s, %s)" % (b[i][0], b[j][0]) for i in range(len(b)) for j in range(i + 1, len(b))
                 if b[i][2] or b[j][2]]
        if pairs:
            L += [I12 + "let (%s) = (%s);" % (", ".join(v[0] for v in b), ", ".join(v[1] for v in b)),
                  I12 + "if !(%s) {" % " && ".join(pairs), I16 + "return PRUV_BAD_ARGS;", I12 + "}"]
        lf = c_len_first(params)
        if lf:
            L += [I12 + "if !(%s) {" % lf, I16 + "return PRUV_BAD_ARGS;", I12 + "}"]
        args = ["&mut st[..]" if smode == "mt" else "&st[..]"]
        for x, k in params:
            if k in _C_ARRAY_KINDS:
                L += [I12 + "let %s = match %s {" % (x, _C_CONV[k] % (x, x)), I16 + "Some(v) => v,",
                      I16 + "None => return PRUV_BAD_ARGS,", I12 + "};"]
                args.append(x)
            elif k == "bool":
                args.append("%s != 0" % x)
            else:
                args.append(x)
        L.append(I12 + "match hd.store.with(|st| super::%s(%s)) {" % (name, ", ".join(args)))
        if rk == "unit":
            L.append(I16 + "Ok(()) => PRUV_OK,")
        else:
            L += [I16 + "Ok(v) => {", I16 + "    *ret = %s;" % ("v" if rk == "i64" else "v as i64"),
                  I16 + "    PRUV_OK", I16 + "}"]
        L += [I16 + "Err(_) => PRUV_POISONED,", I12 + "}", "        })", "    }"]
        parts.append("\n".join(L))
        if smode == "mt" and rk == "i64":
            # P10H: the batched entry: n items in one store access
            sigb = ["h: *mut PruvHandle", "n: i64"]
            reads, rnames, tabs = [], [], ["tab(done as *const i64, 1)", "tab(ret as *const i64, n)"]
            for x, k in params:
                if k == "i64" or k == "bool":
                    sigb.append("%s: *const i64" % x)
                    reads.append("%s.add(i).read()" % x)
                    rnames.append(x)
                    tabs.append("tab(%s, n)" % x)
                else:
                    p = _C_PTR[k]
                    sigb += ["%s: *const %s" % (x, p), "%s_len: *const i64" % x]
                    reads += ["%s.add(i).read()" % x, "%s_len.add(i).read()" % x]
                    rnames += [x, "%s_len" % x]
                    tabs += ["tab(%s, n)" % x, "tab(%s_len, n)" % x]
            sigb += ["ret: *mut i64", "done: *mut i64"]
            # fixed line breaks (independent of the entry name)
            wrapped = ["    /// C batch for `%s`: runs items 0..n in order inside one store access (one crossing, one panic" % name,
                       "    /// guard) and stops after the first item whose result is nonzero. Item i takes aK[i] (and aK_len[i]) for",
                       "    /// each parameter and its result goes to ret[i]. On PRUV_OK or PRUV_BAD_ARGS *done is the number of items",
                       "    /// that ran (0 when the whole call is refused up front); a bad item i is refused before it runs (items",
                       "    /// before it ran). An up-front refusal writes *done = 0 only if done is non-null, aligned, and clear of the",
                       "    /// handle object and of the ret table (ret .. ret + 8 * max(n, 0) bytes); otherwise done is not written. A",
                       "    /// NULL handle (PRUV_BAD_ARGS), a poisoned handle (PRUV_POISONED) and a panic (PRUV_PANICKED) never write",
                       "    /// *done. Tables are read with raw reads, so an item buffer may overlap a table."]
            LB = wrapped + ["    /// # Safety",
                  "    /// The pruv_c caller contract (module docs) holds for every item; each table has n readable elements.",
                  "    #[no_mangle]",
                  '    pub unsafe extern "C" fn @P@pruv_hb_%s(%s) -> i32 {' % (name, ", ".join(sigb)),
                  "        guard(|| unsafe {",
                  I12 + "let hd = match h.as_mut() {", I16 + "Some(h) => h,", I16 + "None => return PRUV_BAD_ARGS,",
                  I12 + "};",
                  I12 + "if hd.store.is_poisoned() {", I16 + "return PRUV_POISONED;", I12 + "}",
                  I12 + "let (sp_rett, sp_done, sp_h) = (span(ret, n), span(done, 1), span(h as *const PruvHandle, 1));",
                  I12 + "if !(%s && apart(sp_rett, sp_done) && apart(sp_rett, sp_h) && apart(sp_done, sp_h)) {"
                  % " && ".join(tabs),
                  I16 + "// up-front refusal: *done = 0 (no item ran) when done itself is a valid slot clear of the handle and the ret table",
                  I16 + "if tab(done as *const i64, 1) && apart(sp_done, sp_rett) && apart(sp_done, sp_h) {",
                  I20 + "done.write(0);", I16 + "}",
                  I16 + "return PRUV_BAD_ARGS;", I12 + "}",
                  I12 + "match hd.store.with(|st| {",
                  I16 + "let mut i: usize = 0;",
                  I16 + "while i < n as usize {"]
            if rnames:
                # a one-element tuple needs its trailing comma
                tc = "," if len(rnames) == 1 else ""
                LB.append(I20 + "let (%s%s) = (%s%s);" % (", ".join(rnames), tc, ", ".join(reads), tc))
            for x, k in params:
                if k == "bool":
                    LB += [I20 + "if %s != 0 && %s != 1 {" % (x, x), I20 + "    return (i, PRUV_BAD_ARGS);", I20 + "}"]
            bb = [("sp_%s" % x, "span(%s, %s_len)" % (x, x), k.startswith("mt_")) for x, k in params
                  if k in _C_ARRAY_KINDS]
            own = [v[0] for v in bb]
            bb += [("sp_rett", None, True), ("sp_done", None, True), ("sp_h", None, True)]
            # the ret table, done and the handle are checked apart once before the loop: only pairs with an item buffer
            pairsb = ["apart(%s, %s)" % (bb[i][0], bb[j][0]) for i in range(len(own)) for j in range(i + 1, len(bb))
                      if bb[i][2] or bb[j][2]]
            if own:
                tc = "," if len(own) == 1 else ""
                LB.append(I20 + "let (%s%s) = (%s%s);" % (", ".join(own), tc,
                                                          ", ".join(v[1] for v in bb[:len(own)]), tc))
                LB += [I20 + "if !(%s) {" % " && ".join(pairsb), I20 + "    return (i, PRUV_BAD_ARGS);", I20 + "}"]
            lf = c_len_first(params)
            if lf:
                LB += [I20 + "if !(%s) {" % lf, I20 + "    return (i, PRUV_BAD_ARGS);", I20 + "}"]
            argsb = ["&mut st[..]"]
            for x, k in params:
                if k in _C_ARRAY_KINDS:
                    LB += [I20 + "let %s = match %s {" % (x, _C_CONV[k] % (x, x)), I20 + "    Some(v) => v,",
                           I20 + "    None => return (i, PRUV_BAD_ARGS),", I20 + "};"]
                    argsb.append(x)
                elif k == "bool":
                    argsb.append("%s != 0" % x)
                else:
                    argsb.append(x)
            LB += [I20 + "let v = super::%s(%s);" % (name, ", ".join(argsb)),
                   I20 + "ret.add(i).write(v);", I20 + "i += 1;", I20 + "if v != 0 {", I20 + "    break;", I20 + "}",
                   I16 + "}", I16 + "(i, PRUV_OK)", I12 + "}) {",
                   I16 + "Ok((k, s)) => {", I20 + "done.write(k as i64);", I20 + "s", I16 + "}",
                   I16 + "Err(_) => PRUV_POISONED,", I12 + "}", "        })", "    }"]
            parts.append("\n".join(LB))
        if smode == "ro" and len(params) == 1 and params[0][1] == "mt_i64" and rk == "i64":
            parts.append("""\
    /// C rebuild for `%s`: runs it from the handle's state into a fresh zeroed buffer of new_len words and, when
    /// it returns 0, makes that buffer the handle's state (the old one is freed); otherwise the state is unchanged.
    /// # Safety
    /// The pruv_c caller contract (module docs) holds.
    #[no_mangle]
    pub unsafe extern "C" fn @P@pruv_hr_%s(h: *mut PruvHandle, new_len: i64, ret: *mut i64) -> i32 {
        guard(|| unsafe {
            let hd = match h.as_mut() {
                Some(h) => h,
                None => return PRUV_BAD_ARGS,
            };
            if hd.store.is_poisoned() {
                return PRUV_POISONED;
            }
            if ret.is_null() || (ret as usize) %% ::std::mem::align_of::<i64>() != 0 || (ret as usize).checked_add(::std::mem::size_of::<i64>()).is_none() {
                return PRUV_BAD_ARGS;
            }
            if !len_ok::<i64>(new_len) {
                return PRUV_BAD_ARGS;
            }
            if !apart(span(ret, 1), span(h as *const PruvHandle, 1)) {
                return PRUV_BAD_ARGS;
            }
            match hd.store.with(|st| {
                let mut next = ::std::vec![0i64; new_len as usize];
                let r = super::%s(&st[..], &mut next[..]);
                if r == 0 {
                    *st = next;
                }
                r
            }) {
                Ok(v) => {
                    *ret = v;
                    PRUV_OK
                }
                Err(_) => PRUV_POISONED,
            }
        })
    }""" % (name, name, name))
    parts.append("}")
    return "\n".join(parts).replace("@P@", pkg + "__")


def gen_bindings(prog, shim=None, verified=True):
    """shim: the `mod pruv_g6_shim` text of a G6 program (foreign_shim.build_shim); None for every other program"""
    tt = new_tt(prog)
    sp = sym_prefix(getattr(prog, "pkg_name", None))   # P10H: prefix of the pruv_abi_* symbols (same as Mod.sym_pre)
    explicit = bool(getattr(prog, "explicit", False))
    raw = getattr(prog, "raw_entries", None) if explicit else None   # P09
    raw_items = []
    o = []
    o.append("// generated by pruvc llvmgen -- Rust bindings for the LLVM back end (include with #[path] mod sol;)")
    if prog.types:
        if explicit:
            # G4 A3 (docs/G4_SPEC.md 7): only the nominal types the host entry signatures depend on, transitively
            # (declaration order of the type table is kept)
            need = entry_type_closure(prog, tt)
            pub = codegen.emit_public_types(prog, [d for d in tt.order if d.name in need]) if need else ""
        else:
            pub = codegen.emit_public_types(prog)
        if pub:
            o.append(pub.replace("#[derive", ALLOW + "\n#[derive") if "#[derive" in pub else pub)
    o.append(ALLOW)
    o.append("#[repr(C)]\npub struct Pruv__Array { pub data: *mut i64, pub len: i64 }")
    if uses_array_type(prog, "array_u8"):
        o.append(ALLOW)
        o.append("#[repr(C)]\npub struct Pruv__ArrayU8 { pub data: *mut u8, pub len: i64 }")
    if uses_array_type(prog, "array_bool"):  # G2 A5
        o.append(ALLOW)
        o.append("#[repr(C)]\npub struct Pruv__ArrayBool { pub data: *mut u8, pub len: i64 }")
    if uses_str(prog, cost=not verified):  # G2 B2
        o.append(ALLOW)
        o.append("#[repr(C)]\npub struct Pruv__Str { pub data: *mut u8, pub len: i64 }")
    o.append(ALLOW)
    o.append("#[repr(C)]\npub struct Pruv__Tagged { pub tag: i64, pub value: i64 }")
    # P3: the host helpers are called only by runtime.c, which has already checked every argument: alloc gets
    # len > 0 within the runtime's bound and a valid layout, free / realloc get a non-null block with its own layout
    # (pruv_free / pruv_free_v2 / pruv_node_free_v1 / pruv_shrink_v2 skip NULL and len <= 0). So the layout is built
    # unchecked; a null result still goes to handle_alloc_error.
    o.append(ALLOW)
    o.append("""\
#[no_mangle]
pub unsafe extern "C" fn pruv_host_alloc(len: i64) -> *mut i64 {
    let layout = unsafe { ::std::alloc::Layout::from_size_align_unchecked((len as usize) * 8, 8) };
    let p = unsafe { ::std::alloc::alloc(layout) } as *mut i64;
    if p.is_null() { ::std::alloc::handle_alloc_error(layout); }
    p
}""")
    o.append(ALLOW)
    o.append("""\
#[no_mangle]
pub unsafe extern "C" fn pruv_host_alloc_zeroed(len: i64) -> *mut i64 {
    let layout = unsafe { ::std::alloc::Layout::from_size_align_unchecked((len as usize) * 8, 8) };
    let p = unsafe { ::std::alloc::alloc_zeroed(layout) } as *mut i64;
    if p.is_null() { ::std::alloc::handle_alloc_error(layout); }
    p
}""")
    o.append(ALLOW)
    o.append("""\
#[no_mangle]
pub extern "C" fn pruv_host_flush() {
    let _ = ::std::io::Write::flush(&mut ::std::io::stdout());
}""")
    o.append(ALLOW)
    o.append("""\
#[no_mangle]
pub unsafe extern "C" fn pruv_host_free(data: *mut i64, len: i64) {
    unsafe { ::std::alloc::dealloc(data as *mut u8, ::std::alloc::Layout::from_size_align_unchecked((len as usize) * 8, 8)) };
}""")
    o.append(ALLOW)
    o.append("""\
#[no_mangle]
pub unsafe extern "C" fn pruv_host_alloc_v2(len: i64, size: i64, align: i64) -> *mut u8 {
    let layout = unsafe { ::std::alloc::Layout::from_size_align_unchecked((len as usize) * (size as usize), align as usize) };
    let p = unsafe { ::std::alloc::alloc(layout) };
    if p.is_null() { ::std::alloc::handle_alloc_error(layout); }
    p
}""")
    o.append(ALLOW)
    o.append("""\
#[no_mangle]
pub unsafe extern "C" fn pruv_host_alloc_zeroed_v2(len: i64, size: i64, align: i64) -> *mut u8 {
    let layout = unsafe { ::std::alloc::Layout::from_size_align_unchecked((len as usize) * (size as usize), align as usize) };
    let p = unsafe { ::std::alloc::alloc_zeroed(layout) };
    if p.is_null() { ::std::alloc::handle_alloc_error(layout); }
    p
}""")
    o.append(ALLOW)
    o.append("""\
#[no_mangle]
pub unsafe extern "C" fn pruv_host_free_v2(data: *mut u8, len: i64, size: i64, align: i64) {
    unsafe { ::std::alloc::dealloc(data, ::std::alloc::Layout::from_size_align_unchecked((len as usize) * (size as usize), align as usize)) };
}""")
    o.append(ALLOW)
    o.append("""\
#[no_mangle]
pub unsafe extern "C" fn pruv_host_realloc_v2(data: *mut u8, len: i64, m: i64, size: i64, align: i64) -> *mut u8 {
    let layout = unsafe { ::std::alloc::Layout::from_size_align_unchecked((len as usize) * (size as usize), align as usize) };
    let p = unsafe { ::std::alloc::realloc(data, layout, (m as usize) * (size as usize)) };
    if p.is_null() { ::std::alloc::handle_alloc_error(layout); }
    p
}""")
    # G3: specializations of generic functions are private (no ABI symbol, not callable from the bindings)
    # G6 A3: trusted declarations and collection-bearing functions have no binding
    # G9 A3: a function with an extended type in its signature has no binding either
    pubs = [f for f in prog.functions if exports_abi(f, tt, explicit)]   # G4: entries only; inout: explicit host entries only
    returns_array = any(is_arr(f.ret) for f in pubs)  # G2 B2: str counts
    decls = []
    for f in pubs:
        if mangle(f.name) != f.name:
            raise Exception("llvmgen: function name %s needs mangling; the bindings use the raw name" % f.name)
        ps = []
        for p in f.params:
            if p.mode == "inout":   # mutable host entry: the borrowed buffer is written in place
                ps += ["*mut u8" if p.type.name in ("array_u8", "array_bool") else "*mut i64", "i64"]
            elif p.type.name in ("array_u8", "array_bool", "str"):  # G2 A5/B2
                ps += ["*const u8", "i64"]
            elif p.type == ARRAY:
                ps += ["*const i64", "i64"]
            elif p.type in (OPTION, RESULT):
                ps += ["i64", "i64"]
            elif is_agg(p.type):
                ps.append("*const i64")
            else:
                ps.append("i64")
        if f.ret.name == "array_u8":
            ps.append("*mut Pruv__ArrayU8")
        elif f.ret.name == "array_bool":  # G2 A5
            ps.append("*mut Pruv__ArrayBool")
        elif f.ret.name == "str":  # G2 B2
            ps.append("*mut Pruv__Str")
        elif f.ret == ARRAY:
            ps.append("*mut Pruv__Array")
        elif is_agg(f.ret):
            ps.append("*mut i64")
        ret = " -> i64" if f.ret in (I64, BOOL) else (" -> Pruv__Tagged" if f.ret in (OPTION, RESULT) else "")
        decls.append("    fn %spruv_abi_%s(%s)%s;" % (sp, typed_abi_name(f), ", ".join("p%d: %s" % (i, t) for i, t in enumerate(ps)), ret))
    if any(f.ret.name == "array_bool" for f in pubs):  # G2 A5
        decls.append("    fn pruv_check_bool(p0: *const u8, p1: i64);")
    if any(f.ret.name == "str" for f in pubs):  # G2 B2
        decls.append("    fn pruv_check_utf8(p0: *const u8, p1: i64);")
    if returns_array:
        decls.append("    fn pruv_runtime_rust_allocator_v2();")
    o.append(ALLOW)
    o.append('extern "C" {\n%s\n}' % "\n".join(decls))
    for f in pubs:
        ps = ", ".join("%s: %s" % (p.name, ("&mut " + rust_ty(p.type, True)[1:]) if p.mode == "inout"
                                   else rust_ty(p.type, True)) for p in f.params)
        ret = "" if f.ret == UNIT else " -> " + rust_ty(f.ret, False)
        pre, args = [], []
        for p in f.params:
            if p.mode == "inout":
                args += ["%s.as_mut_ptr()%s" % (p.name, " as *mut u8" if p.type.name == "array_bool" else ""),
                         "%s.len() as i64" % p.name]
            elif p.type == I64:
                args.append(p.name)
            elif p.type == BOOL:
                args.append("%s as i64" % p.name)
            elif p.type.name == "array_bool":  # G2 A5
                args += ["%s.as_ptr() as *const u8" % p.name, "%s.len() as i64" % p.name]
            elif p.type.name == "str":  # G2 B2
                args += ["%s.as_ptr()" % p.name, "%s.len() as i64" % p.name]
            elif is_array(p.type):
                args += ["%s.as_ptr()" % p.name, "%s.len() as i64" % p.name]
            elif is_agg(p.type):
                w = "pruv__w_" + p.name
                pre.append("let mut %s = [0i64; %d];" % (w, words(tt, p.type)))
                pre += rs_ser(tt, p.type, p.name, w, 0, [0])
                args.append(w + ".as_ptr()")
            else:
                a, b = ("Some", "None") if p.type == OPTION else ("Ok", "Err")
                pre.append("let (pruv__t_%s, pruv__v_%s): (i64, i64) = match %s { %s(v) => (1, v), %s(v) => (0, v) };"
                           % (p.name, p.name, p.name, a, b))
                if p.type == OPTION:
                    pre[-1] = ("let (pruv__t_%s, pruv__v_%s): (i64, i64) = match %s { Some(v) => (1, v), None => (0, 0) };"
                               % (p.name, p.name, p.name))
                args += ["pruv__t_" + p.name, "pruv__v_" + p.name]
        body = []
        if "io" in f.effects:
            body.append("::std::io::Write::flush(&mut ::std::io::stdout());")
        if is_arr(f.ret):
            body.append("let _ = ::std::hint::black_box(pruv_runtime_rust_allocator_v2 as unsafe extern \"C\" fn());")
        body += pre
        call = "%spruv_abi_%s(%s)" % (sp, typed_abi_name(f), ", ".join(args))
        callo = "%spruv_abi_%s(%s)" % (sp, typed_abi_name(f), ", ".join(args + ["&mut pruv__out"]))
        if f.ret == UNIT:
            body.append(call + ";")
        elif f.ret == I64:
            body.append(call)
        elif f.ret == BOOL:
            body.append(call + " != 0")
        elif f.ret.name == "array_bool":  # G2 A5
            body.append("let mut pruv__out = Pruv__ArrayBool { data: ::std::ptr::null_mut(), len: 0 };")
            body.append(callo + ";")
            body.append("pruv_check_bool(pruv__out.data, pruv__out.len);")
            body.append("if pruv__out.len > 0 { ::std::vec::Vec::from_raw_parts(pruv__out.data as *mut bool, pruv__out.len as usize, "
                        "pruv__out.len as usize) } else { ::std::vec::Vec::new() }")
        elif f.ret.name == "str":  # G2 B2
            body.append("let mut pruv__out = Pruv__Str { data: ::std::ptr::null_mut(), len: 0 };")
            body.append(callo + ";")
            body.append("pruv_check_utf8(pruv__out.data, pruv__out.len);")
            body.append("if pruv__out.len > 0 { ::std::string::String::from_utf8_unchecked(::std::vec::Vec::from_raw_parts("
                        "pruv__out.data, pruv__out.len as usize, pruv__out.len as usize)) } else { ::std::string::String::new() }")
        elif is_array(f.ret):
            body.append("let mut pruv__out = %s { data: ::std::ptr::null_mut(), len: 0 };"
                        % ("Pruv__ArrayU8" if f.ret.name == "array_u8" else "Pruv__Array"))
            body.append(callo + ";")
            body.append("if pruv__out.len > 0 { ::std::vec::Vec::from_raw_parts(pruv__out.data, pruv__out.len as usize, "
                        "pruv__out.len as usize) } else { ::std::vec::Vec::new() }")
        elif is_agg(f.ret):
            body.append("let mut pruv__out = [0i64; %d];" % words(tt, f.ret))
            body.append("%spruv_abi_%s(%s);" % (sp, typed_abi_name(f), ", ".join(args + ["pruv__out.as_mut_ptr()"])))
            body.append(rs_de(tt, f.ret, "pruv__out", 0))
        else:
            a, b = ("Some", "None") if f.ret == OPTION else ("Ok", "Err")
            body.append("let pruv__out = %s;" % call)
            if f.ret == OPTION:
                body.append("if pruv__out.tag != 0 { Some(pruv__out.value) } else { None }")
            else:
                body.append("if pruv__out.tag != 0 { Ok(pruv__out.value) } else { Err(pruv__out.value) }")
        o.append(ALLOW)
        o.append("#[inline]\n%sfn %s(%s)%s {\n    unsafe {\n%s\n    }\n}" %
                 ("pub(crate) " if raw else "pub ", f.name, ps, ret, "\n".join("        " + b for b in body)))
        if raw:
            raw_items.append((f.name, ps, ret, [p.name for p in f.params]))
    if raw:   # P09: manifest raw_entries; the wrappers above are pub(crate)
        o.append(codegen.raw_module("crate" if raw == "c" else raw, raw_items))
    if raw == "c":   # P10: the C adapter module, after pruv_raw and before the G6 shim
        o.append(c_module(c_entries(pubs), getattr(prog, "pkg_name", None)))
    if shim is not None:   # G6 A3: the bridge shim module, exactly once, last
        o.append(shim.rstrip("\n"))
    return "\n".join(o) + "\n"


def _g6_scalar(t):
    return t == I64 or t == BOOL


def _g6_coll_ok(t):
    """G6_SPEC 13.1 collections, plus the G10 File handle."""
    if t.name == "vec":
        return _g6_scalar(t.args[0])
    if t.name == "hashmap":
        return _g6_scalar(t.args[0]) and _g6_scalar(t.args[1])
    return t.name in ("string", "file", "process", "process_args", "socket", "listener", "loop")


def _g6_type_ok(t):
    """a type that may occur in a G6 program on LLVM: no collection, or a supported one (never nested)"""
    if is_collection(t):
        return _g6_coll_ok(t)
    if is_owned_result(t):     # LG12 E2: the String error is the only owned part
        return not contains_collection(t.args[0])
    return not contains_collection(t)


def g6_llvm_error(prog):
    """PruvError codegen/unsupported_type for the first G6 construct outside G6_SPEC 13.1, or None.  Source order:
    functions in order, the signature before the body, inside a body the earliest position."""
    if first_g6_use(prog) is None:
        return None

    def needs_rust(t):
        return (t.name == "hashmap" and t.args[0].name == "string"
                or t.name == "vec" and t.args[0].name == "string"
                or any(needs_rust(a) for a in t.args))

    def err(line, col, what=None):
        msg = G6_UNSUPPORTED_MSG if what is None else "%s needs the Rust backend" % what
        fix = G6_UNSUPPORTED_FIX if what is None else "use the Rust backend"
        e = PruvError("codegen", line, col, "unsupported_type", msg, fix)
        e.file = getattr(f, "file", None)   # explicit mode: the function's source file (Rust: the tagged line)
        return e

    funcs = {f.name: f for f in prog.functions}
    for f in prog.functions:
        if f.foreign is not None:
            if needs_rust(f.ret):
                return err(f.line, f.col, "a trusted Vec<String> result")
            ok = (f.ret in (I64, BOOL, UNIT) or _owned_result(f.ret)) and all(
                p.type in (I64, BOOL, ARRAY, ARRAY_U8, ARRAY_BOOL, STR) and not p.take for p in f.params)
            if not ok:
                return err(f.line, f.col)
            continue
        if any(needs_rust(t) for t in [f.ret] + [p.type for p in f.params]):
            return err(f.line, f.col, "this collection type")
        if any(not _g6_type_ok(p.type) for p in f.params) or not _g6_type_ok(f.ret):
            return err(f.line, f.col)
        bad = []
        rust_only = []

        def visit(n):
            if isinstance(n, ForMap):
                rust_only.append((n.line, n.col, "map iteration"))
            if isinstance(n, Let) and not _g6_type_ok(n.type):
                bad.append((n.line, n.col))
            ty = getattr(n, "ty", None)
            if ty is not None and needs_rust(ty):
                rust_only.append((n.line, n.col, "this collection type"))
            if ty is not None and not _g6_type_ok(ty):
                bad.append((n.line, n.col))
            if isinstance(n, Call) and n.name in funcs and any(
                    is_vec_slice_arg(a.ty, p.type) for a, p in zip(n.args, funcs[n.name].params)):
                rust_only.append((n.line, n.col, "a borrowed vector slice argument"))
            if isinstance(n, Call) and n.name in CATALOG and n.name not in CATALOG_CONTRACT_ONLY:
                if (n.name in ("map_remove", "map_take", "vec_at_mut", "vec_clear", "vec_extend_from_slice", "string_eq")
                        or (n.name == "vec_reserve" and n.inst["args"][0] not in (I64, BOOL))):
                    rust_only.append((n.line, n.col, n.name))
                if n.inst and not all(_g6_scalar(a) for a in n.inst["args"]):
                    bad.append((n.line, n.col))

        foreign_shim._walk(f.body, visit)
        if rust_only:
            return err(*min(rust_only))
        if bad:
            line, col = min(bad)
            return err(line, col)
    return None


def _is_coll_fn(f):
    return any(is_collection(p.type) for p in f.params) or is_collection(f.ret)


def _result_ok_value(t, extended):
    if t.owner is not None:
        return False
    if _g6_scalar(t) or t in (OPTION, RESULT):
        return True
    return (gopt(t) or (is_user_type(t) and t.name not in extended)) and all(
        _result_ok_value(a, extended) for a in t.args)


def _typed_result_in(node, specialized_args, extended, in_decl=False):
    """the first type the llvm layer cannot lay out: an owned Result<T, String> (LG12 E2) is supported as a local,
    parameter, result or temporary with a value T; as a record / enum field or an argument of another type it
    is not (it would need a drop path through that type)"""
    if isinstance(node, Type):
        if is_owned_result(node):
            if in_decl or not _result_ok_value(node.args[0], extended):
                return node
        elif any(is_owned_result(a) for a in node.args):
            return node
        # Lowered nominal applications keep their arguments in the specialization plan.
        children = node.args or specialized_args.get(node.name, ())
    elif isinstance(node, (list, tuple)):
        children = node
    elif dataclasses.is_dataclass(node):
        # an expression's own type, type arguments and specialization come before its operands (as in Rust)
        children = [getattr(node, "ty", None), getattr(node, "targs", None)]
        if isinstance(node, Call):
            children.extend(specialized_args.get(node.name, ()))
        children.extend(getattr(node, field.name) for field in dataclasses.fields(node) if field.name != "ty")
        children.extend(getattr(node, attr, None) for attr in ("cost", "decreases"))
    else:
        return None
    for child in children:
        hit = _typed_result_in(child, specialized_args, extended, in_decl)
        if hit is not None:
            return hit
    return None


def gen_llvm(prog, verified=True, trust_callers=False, budget=1 << 30, plan=None, foreign_checks="requires"):
    """plan: a mono.SpecPlan for a generic program (computed here when omitted; PruvError specialization_limit
    propagates).  The generator works on the concrete plan.program only: no type parameter reaches LLVM.
    foreign_checks: G6 boundary check mode of the shim helpers (requires|all|off, docs/G6_SPEC.md 12.3).
    A G6 program outside the supported subset (G6_SPEC 13.1) raises PruvError codegen/unsupported_type."""
    from typeinfo import g2_unimplemented_error, first_g3_use, type_name
    if foreign_checks not in ("requires", "all", "off"):
        raise ValueError("foreign_checks must be requires, all or off")
    from check import opaque_inline
    opaque_inline(prog)     # P13o: contract applications of opaque functions become their bodies
    for f in prog.functions:
        if f.host_entry and has_take_array(f):
            raise PruvError("codegen", f.line, f.col, "unsupported_type",
                            "%s has a take array parameter, which needs the Rust backend" % getattr(f, "disp", f.name),
                            "use the Rust backend", file=getattr(f, "file", None))
        if any(p.mode == "inout" and not is_array(p.type) and p.type not in (I64, BOOL)
               and (f.host_entry or not (is_agg(p.type) or is_scalar_vec(p.type)))     # G-LG7-1: internal inout Vec<i64>/Vec<bool> passes its handle
               for p in f.params):
            kind = "struct"
            if any(p.mode == "inout" and is_collection(p.type) for p in f.params):
                kind = "collection"
            raise PruvError("codegen", f.line, f.col, "unsupported_type",
                            "%s has an inout %s parameter, which needs the Rust backend" %
                            (getattr(f, "disp", f.name), kind),
                            "use the Rust backend", file=getattr(f, "file", None))
    for d in prog.types:
        if any(is_collection(fd.type) for fd in TypeTable._all_fields(d)):
            raise PruvError("codegen", d.line, d.col, "unsupported_type",
                            "%s has owned collection fields, which need the Rust backend" % getattr(d, "disp", d.name),
                            "use the Rust backend",
                            file=getattr(d, "file", None))
    u8_structs = {d.name for d in prog.types if isinstance(d, StructDecl) and any(fd.type == ARRAY_U8 for fd in d.fields)}
    for f in prog.functions:
        if f.host_entry and any(p.type.owner is None and p.type.name in u8_structs for p in f.params):
            raise PruvError("codegen", f.line, f.col, "unsupported_type",     # R-6f-6
                            "host entry %s takes a struct with a [u8] field, which has no C ABI" % getattr(f, "disp", f.name),
                            "use the Rust backend", file=getattr(f, "file", None))
        if f.host_entry and any(p.take and p.type == Type("string") for p in f.params):
            raise PruvError("codegen", f.line, f.col, "unsupported_type",
                            "host entry %s takes an owned String, which has no C ABI" % getattr(f, "disp", f.name),
                            "use the Rust backend", file=getattr(f, "file", None))
        if f.host_entry and any(is_owned_result(t) for t in [f.ret] + [p.type for p in f.params]):
            raise PruvError("codegen", f.line, f.col, "unsupported_type",     # LG12 E2
                            "host entry %s has a Result<_, String> in its signature, which has no C ABI" % getattr(f, "disp", f.name),
                            "use the Rust backend", file=getattr(f, "file", None))
        if f.host_entry and is_scalar_vec(f.ret):
            raise PruvError("codegen", f.line, f.col, "unsupported_type",
                            "host entry %s returns a Vec, which has no C ABI" % getattr(f, "disp", f.name),
                            "use the Rust backend",
                            file=getattr(f, "file", None))
    if getattr(prog, "contract_vec_slice", None) is not None:
        line, col, file = prog.contract_vec_slice
        raise PruvError("codegen", line, col, "unsupported_type",
                        "a borrowed vector slice argument needs the Rust backend", "use the Rust backend", file=file)
    if plan is None and (first_g3_use(prog) is not None or prog.explicit):   # G4 A3: explicit mode names via mono
        from mono import specialize
        plan = specialize(prog)
    source = prog if plan is None else plan.source
    specialized_args = {}
    if plan is not None:
        prog = plan.program
        specialized_args = {spec.stem: spec.args for spec in (*plan.type_specs, *plan.fn_specs)}
    positions = {(item.file, item.line, item.col): i for i, item in enumerate(source.items or ())}
    items = sorted([*prog.types, *prog.functions],
                   key=lambda item: positions.get((item.file, item.line, item.col), len(positions)))
    decl_ids = {id(d) for d in prog.types}
    extended, _ = codegen.g9_sets(prog)
    for item in items:
        hit = _typed_result_in(item, specialized_args, extended, id(item) in decl_ids)
        if hit is not None:
            raise PruvError("codegen", item.line, item.col, "unsupported_type",
                            ("%s uses %s, an owned String error Result that the llvm codegen layer supports only with a "
                             "value Ok type (i64, bool, a value struct or enum, or an Option of one), outside records "
                             "and other generic types" if is_owned_result(hit) else
                             "%s uses %s, a Result with a non-i64 error type, which the llvm codegen layer does not "
                             "support yet") % (getattr(item, "disp", item.name), type_name(hit)),
                            "use the Rust backend", file=item.file)
    explicit = bool(getattr(prog, "explicit", False))
    g6err = g6_llvm_error(prog)
    if g6err is not None:
        raise g6err
    is_g6 = first_g6_use(prog) is not None
    par_targets = set()
    def find_par(node):
        if isinstance(node, Par):
            par_targets.update(c.name for c in node.calls)
    for f in prog.functions:
        foreign_shim._walk(f.body, find_par)
    g2err = g2_unimplemented_error(prog, "codegen", skip=("array_u8", "array_bool", "str"))  # G2 B2
    if g2err is not None:  # same as the Rust generator's Err(String); drivers reject earlier with the diagnostic
        raise RuntimeError("unsupported_type: %s" % g2err.message)
    tt = new_tt(prog)   # G9: SCC-condensed order and the extended / boxed metadata (identical to TypeTable otherwise)
    m = Mod(tt)
    m.array_resizes = array_resize_params(prog)
    for f in prog.functions:
        for i, p in enumerate(f.params):
            if (f.name, i) in m.array_resizes:
                p.resizable = True
    m.sym_pre = sym_prefix(getattr(prog, "pkg_name", None))
    m.storage = plan_storage(prog, verified)
    from stack import plan_stack
    m.stack = plan_stack(prog, m.storage, verified)
    narrowed = any(w != "i64" for _, w in m.storage.entries)
    m.explicit = explicit
    cost_used = not verified and any(f.cost is not None for f in prog.functions)
    funcs = {f.name: f for f in prog.functions}
    chunks = []

    def checked_entry(f):
        """G4 A3 (docs/G4_SPEC.md 4, same policy as codegen.Gen): the functions that get a checked `__pruv_entry`.
        Verified: only host entries with a requires (none under trust_callers); safe mode: every function with a
        requires, also under trust_callers (it removes only the host-boundary checks)"""
        return bool(f.requires) and f.foreign is None and (not verified or (f.host_entry and not trust_callers))

    if explicit:
        need_cdiv = any(checked_entry(f) for f in prog.functions)
    else:
        need_cdiv = any(f.requires and f.foreign is None and (not trust_callers or (not verified and has_inout(f)))
                        for f in prog.functions)    # G5 A3: an inout entry checks also under trust_callers
    need_cdiv = need_cdiv or (not verified and any(f.requires and f.name in par_targets for f in prog.functions))
    if need_cdiv or cost_used:
        chunks.append(cdivrem(m, True))
        chunks.append(cdivrem(m, False))
    for f in prog.functions:
        if f.foreign is not None:   # G6 A3: a trusted rust fn has no body; its calls go to the shim adapter
            continue
        # G6 A3: collection-bearing functions are internal (no ABI wrapper, no binding); G9 A3: so are functions with
        # an extended type in their signature
        coll = _is_coll_fn(f) or is_internal_fn(f, tt)
        g = FnGen(m, funcs, f, verified)
        g.body()
        if g.cost:
            chunks.append(entry_of(m, funcs, f, trust_callers, budget, cost=True))
        params = ir_params(f, tt)
        chunks.append(g.render("define internal %s @%s__pruv_body(%s)" % (ir_ret(tt, f), mangle(f.name), sig_text(params, f))))
        if not verified and f.requires and f.name in par_targets:
            chunks.append(entry_of(m, funcs, f, False, budget).replace(
                "@%s__pruv_entry(" % mangle(f.name), "@%s__pruv_par_chk(" % mangle(f.name)))
        if explicit:
            # G4 A3: bodies and checked wrappers are internal; only a host entry gets pruv_abi_* (and its typed variant).
            # The host boundary calls the checked wrapper unless trust_callers skips the check (or there is no requires).
            if checked_entry(f):
                chunks.append(entry_of(m, funcs, f, False, budget, verified=verified))
            if is_host_abi_fn(f, tt, True):
                chunks.append(abi_of(f, tt, m, "entry" if (checked_entry(f) and not trust_callers) else "body"))
            continue
        if not (coll and verified):   # verified callers call the body; the entry only serves safe-mode callers
            # G5 A3: the entry of an inout function is internal and always checked (trust_callers does not waive it)
            chunks.append(entry_of(m, funcs, f, (trust_callers and not has_inout(f)) or not f.requires, budget,
                                   verified=verified))
        if not getattr(f, "private", False) and not coll:  # G3: a generic specialization has no ABI symbol
            chunks.append(abi_of(f, tt, m))
    if tt.g9_ext or m.need_size or m.need_eq:   # G9 A3: clone / drop / node helpers, then the contract helpers above
        chunks.extend(g9_helper_chunks(m, tt))
    # G9 A3: buffer fields count as uses of the buffer runtime declarations
    has_str = uses_str(prog, cost=not verified) or type_has_field(prog, "str")  # G2 B2: walk the AST once
    g6decls = ""
    if m.g6_bridge or m.g6_decls:   # G6 A3: bridge marker and the shim symbols used, sorted by symbol
        g6decls = G6_BRIDGE_DECL.replace("@", "@" + m.sym_pre, 1) + "".join(m.g6_decls[k] + "\n" for k in sorted(m.g6_decls))
    g9_u8 = type_has_field(prog, "array_u8") or type_has_field(prog, "array_bool")
    ir = m.render_strings() + "\n" + type_defs(tt) + ("\n" if prog.types else "") + DECLS + (
        DECLS_U8 if (uses_array_type(prog, "array_u8") or uses_array_type(prog, "array_bool") or has_str or m.g6_v2
                     or g9_u8 or narrowed)
        else "") + (
        "declare noalias ptr @pruv_alloc_narrow(i64, i64, i64, i64) nounwind\n" if narrowed else "") + (
        DECLS_BOOL if uses_array_type(prog, "array_bool") else "") + (
        DECLS_STR if has_str else "") + (
        DECLS_REPR if m.uses_repr else "") + (
        DECLS_NODE if tt.g9_boxed else "") + "".join(
        "declare i%d @llvm.bswap.i%d(i%d)\n" % (w, w, w) for w in (16, 32, 64) if w in m.bswap) + (
        "declare ptr @pruv_shrink_v2(ptr, i64, i64, i64, i64) nounwind\n" if m.shrink else "") + (
        DECLS_OVERLAP if m.overlap else "") + (
        "declare void @pruv_fill_v2(ptr nocapture, i64, i64, i64) nounwind\n" if m.fill else "") + (
        DECLS_TAIL if "i64" in m.tail else "") + (
        DECLS_TAIL_U8 if "u8" in m.tail else "") + (
        DECLS_MINMAX if m.tail else "") + (
        DECLS_RESIZE if "i64" in m.resize else "") + (
        DECLS_RESIZE_U8 if "u8" in m.resize else "") + (
        DECLS_RESIZE_NARROW if "narrow" in m.resize else "") + (
        DECL_REGROW_V2 if ("regrow_i64" in m.resize or "regrow_u8" in m.resize or "regrow_narrow" in m.resize)
        else "") + (
        DECLS_REGROW if "regrow_i64" in m.resize else "") + (
        DECLS_REGROW_U8 if "regrow_u8" in m.resize else "") + (
        DECLS_REGROW_NARROW if "regrow_narrow" in m.resize else "") + (
        "declare void @llvm.memmove.p0.p0.i64(ptr nocapture writeonly, ptr nocapture readonly, i64, i1 immarg)\n"
        if m.memmove else "") + (
        "declare i32 @memcmp(ptr nocapture, ptr nocapture, i64) nounwind readonly\n" if m.memcmp else "") + (
        g6decls + "\n" + "\n".join(chunks))  # G2 A5/B2, G3, G6 A3, G9 A3; P2 batch 4: llvm.bswap when used
    if m.uses_utf8_valid:
        ir += Path(__file__).with_name("utf8_valid.ll").read_text()
    if m.uses_utf8_panic:
        ir += "declare void @pruv_panic_utf8(i64) noreturn cold nounwind\n"
    if m.par_trampolines:
        ir = "%pruv_g7_job = type { ptr, ptr }\n" + "\n".join(m.par_types) + "\n" + ir
        ir += "declare void @pruv_g7_v1_run(ptr, i64) nounwind\n"
        ir += "\n".join(m.par_trampolines)
    shim = None
    if is_g6:
        shim = foreign_shim.build_shim(prog, verified, trust_callers, budget, foreign_checks, m.g6_take,
                                       sorted(m.g6_unbox), m.sym_pre)
    if first_par(prog) is not None:
        shim = (shim or "") + parallel_shim.build_shim()
    if cost_used:
        ir += "declare void @pruv_panic_cost() noreturn cold nounwind\n"
        ir += "declare { i128, i1 } @llvm.uadd.with.overflow.i128(i128, i128)\n"
        shim = (shim or "") + """
#[no_mangle]
pub extern "C" fn pruv_panic_cost() -> ! {
    eprintln!("pruv: cost exceeded");
    ::std::process::exit(101);
}
"""
    if m.range_nodes:   # P3 batch 2: !range metadata nodes, in number order
        ir += "\n" + "".join("!%d = !{%s}\n" % (n, body) for body, n in sorted(m.range_nodes.items(), key=lambda kv: kv[1]))
    return llvm_inline(ir), gen_bindings(prog, shim, verified)
