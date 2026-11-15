target datalayout = "e-m:w-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-pc-windows-msvc"

@.str.0 = private unnamed_addr constant [22 x i8] c"division by zero in /\00"
@.str.1 = private unnamed_addr constant [19 x i8] c"i128 overflow in /\00"
@.str.2 = private unnamed_addr constant [22 x i8] c"division by zero in %\00"
@.str.3 = private unnamed_addr constant [19 x i8] c"i128 overflow in %\00"
@.str.4 = private unnamed_addr constant [6 x i8] c"bytes\00"
@.str.5 = private unnamed_addr constant [20 x i8] c"(len(a) <= 1000000)\00"

declare noalias ptr @pruv_alloc(i64, i64) nounwind
declare noalias ptr @pruv_copy(ptr nocapture readonly, i64) nounwind
declare void @pruv_free(ptr nocapture, i64) nounwind
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
declare noalias ptr @pruv_alloc_u8(i64, i64) nounwind
declare noalias ptr @pruv_copy_v2(ptr nocapture readonly, i64, i64, i64) nounwind
declare void @pruv_free_v2(ptr nocapture, i64, i64, i64) nounwind
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
declare ptr @pruv_shrink_v2(ptr, i64, i64, i64, i64) nounwind

define internal i128 @pruv_cdiv(i128 %a, i128 %b) {
entry:
  %z = icmp eq i128 %b, 0
  br i1 %z, label %dz, label %c1
dz:
  call void @pruv_panic_undefined(ptr @.str.0)
  unreachable
c1:
  %am = icmp eq i128 %a, -170141183460469231731687303715884105728
  %bm = icmp eq i128 %b, -1
  %ov = and i1 %am, %bm
  br i1 %ov, label %of, label %c2
of:
  call void @pruv_panic_undefined(ptr @.str.1)
  unreachable
c2:
  %q = sdiv i128 %a, %b
  %r = srem i128 %a, %b
  %neg = icmp slt i128 %r, 0
  %bp = icmp sgt i128 %b, 0
  %ra = add i128 %r, %b
  %rs = sub i128 %r, %b
  %r2 = select i1 %bp, i128 %ra, i128 %rs
  %rr = select i1 %neg, i128 %r2, i128 %r
  %qd = select i1 %bp, i128 -1, i128 1
  %q2 = add i128 %q, %qd
  %qq = select i1 %neg, i128 %q2, i128 %q
  ret i128 %qq
}

define internal i128 @pruv_crem(i128 %a, i128 %b) {
entry:
  %z = icmp eq i128 %b, 0
  br i1 %z, label %dz, label %c1
dz:
  call void @pruv_panic_undefined(ptr @.str.2)
  unreachable
c1:
  %am = icmp eq i128 %a, -170141183460469231731687303715884105728
  %bm = icmp eq i128 %b, -1
  %ov = and i1 %am, %bm
  br i1 %ov, label %of, label %c2
of:
  call void @pruv_panic_undefined(ptr @.str.3)
  unreachable
c2:
  %q = sdiv i128 %a, %b
  %r = srem i128 %a, %b
  %neg = icmp slt i128 %r, 0
  %bp = icmp sgt i128 %b, 0
  %ra = add i128 %r, %b
  %rs = sub i128 %r, %b
  %r2 = select i1 %bp, i128 %ra, i128 %rs
  %rr = select i1 %neg, i128 %r2, i128 %r
  %qd = select i1 %bp, i128 -1, i128 1
  %q2 = add i128 %q, %qd
  %qq = select i1 %neg, i128 %q2, i128 %q
  ret i128 %rr
}

define internal { ptr, i64, i64 } @bytes__pruv_body(ptr noalias nocapture readonly %p.a.a, i64 %p.a.n) {
entry:
  %l0.t.a = alloca ptr
  %l0.t.b = alloca i64
  %l0.t.c = alloca i64
  %l1.c.a = alloca ptr
  %l1.c.b = alloca i64
  %l1.c.c = alloca i64
  br label %start
start:
  %t1 = call ptr @pruv_copy_v2(ptr %p.a.a, i64 %p.a.n, i64 1, i64 1)
  store ptr %t1, ptr %l0.t.a
  store i64 %p.a.n, ptr %l0.t.b
  store i64 %p.a.n, ptr %l0.t.c
  %t2 = load ptr, ptr %l0.t.a
  %t3 = load i64, ptr %l0.t.b
  %t4 = load i64, ptr %l0.t.c
  store ptr null, ptr %l0.t.a
  store i64 0, ptr %l0.t.b
  store i64 0, ptr %l0.t.c
  store ptr %t2, ptr %l1.c.a
  store i64 %t3, ptr %l1.c.b
  store i64 %t4, ptr %l1.c.c
  %t5 = load ptr, ptr %l1.c.a
  %t6 = load i64, ptr %l1.c.b
  %t7 = load i64, ptr %l1.c.c
  store ptr null, ptr %l1.c.a
  store i64 0, ptr %l1.c.b
  store i64 0, ptr %l1.c.c
  %t8 = load ptr, ptr %l1.c.a
  %t9 = load i64, ptr %l1.c.b
  %t10 = load i64, ptr %l1.c.c
  call void @pruv_free_v2_nn(ptr %t8, i64 %t10, i64 1, i64 1)
  %t11 = load ptr, ptr %l0.t.a
  %t12 = load i64, ptr %l0.t.b
  %t13 = load i64, ptr %l0.t.c
  call void @pruv_free_v2_nn(ptr %t11, i64 %t13, i64 1, i64 1)
  %t14 = insertvalue { ptr, i64, i64 } undef, ptr %t5, 0
  %t15 = insertvalue { ptr, i64, i64 } %t14, i64 %t6, 1
  %t16 = insertvalue { ptr, i64, i64 } %t15, i64 %t7, 2
  ret { ptr, i64, i64 } %t16
}

define internal { ptr, i64, i64 } @bytes__pruv_entry(ptr noalias nocapture readonly %p.a.a, i64 %p.a.n) {
entry:
  %req.budget = alloca i64
  br label %start
start:
  store i64 1073741824, ptr %req.budget
  %t1 = zext i64 %p.a.n to i128
  %t2 = icmp sle i128 %t1, 1000000
  br i1 %t2, label %ok.1, label %fail.2
fail.2:
  call void @pruv_panic_requires(ptr @.str.4, ptr @.str.5)
  unreachable
ok.1:
  %t3 = call { ptr, i64, i64 } @bytes__pruv_body(ptr %p.a.a, i64 %p.a.n)
  ret { ptr, i64, i64 } %t3
}

define void @pruv_abi_bytes(ptr %a.a.a, i64 %a.a.n, ptr %out) {
entry:
  %r = call { ptr, i64, i64 } @bytes__pruv_entry(ptr %a.a.a, i64 %a.a.n)
  %x0r = extractvalue { ptr, i64, i64 } %r, 0
  %x1 = extractvalue { ptr, i64, i64 } %r, 1
  %x2 = extractvalue { ptr, i64, i64 } %r, 2
  %o0 = getelementptr inbounds { ptr, i64 }, ptr %out, i32 0, i32 0
  %o1 = getelementptr inbounds { ptr, i64 }, ptr %out, i32 0, i32 1
  store i64 %x1, ptr %o1
  %x3 = icmp eq i64 %x2, %x1
  br i1 %x3, label %shr.same, label %shr.do
shr.do:
  %x0 = call ptr @pruv_shrink_v2(ptr %x0r, i64 %x2, i64 %x1, i64 1, i64 1)
  store ptr %x0, ptr %o0
  ret void
shr.same:
  store ptr %x0r, ptr %o0
  ret void
}
