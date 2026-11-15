target datalayout = "e-m:w-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-pc-windows-msvc"

@.str.0 = private unnamed_addr constant [22 x i8] c"division by zero in /\00"
@.str.1 = private unnamed_addr constant [19 x i8] c"i128 overflow in /\00"
@.str.2 = private unnamed_addr constant [22 x i8] c"division by zero in %\00"
@.str.3 = private unnamed_addr constant [19 x i8] c"i128 overflow in %\00"
@.str.4 = private unnamed_addr constant [4 x i8] c"arr\00"
@.str.5 = private unnamed_addr constant [20 x i8] c"(len(a) <= 1000000)\00"

declare noalias nonnull ptr @pruv_raw_alloc_v3(i64, i64, i64) nounwind
declare noalias nonnull ptr @pruv_raw_alloc_zeroed_v3(i64, i64, i64) nounwind
@pruv_alloc_failed_msg = private unnamed_addr constant [18 x i8] c"allocation failed\00"
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

define internal { ptr, i64, i64 } @arr__pruv_body(ptr noalias nocapture readonly %p.a.a, i64 %p.a.n) {
entry:
  %l0.c.a = alloca ptr
  %l0.c.b = alloca i64
  %l0.c.c = alloca i64
  br label %start
start:
  %t1 = call ptr @pruv_copy(ptr %p.a.a, i64 %p.a.n)
  store ptr %t1, ptr %l0.c.a
  store i64 %p.a.n, ptr %l0.c.b
  store i64 %p.a.n, ptr %l0.c.c
  %t2 = load ptr, ptr %l0.c.a
  %t3 = load i64, ptr %l0.c.b
  %t4 = load i64, ptr %l0.c.c
  store ptr null, ptr %l0.c.a
  store i64 0, ptr %l0.c.b
  store i64 0, ptr %l0.c.c
  %t5 = load ptr, ptr %l0.c.a
  %t6 = load i64, ptr %l0.c.b
  %t7 = load i64, ptr %l0.c.c
  call void @pruv_free_nn(ptr %t5, i64 %t7)
  %t8 = insertvalue { ptr, i64, i64 } undef, ptr %t2, 0
  %t9 = insertvalue { ptr, i64, i64 } %t8, i64 %t3, 1
  %t10 = insertvalue { ptr, i64, i64 } %t9, i64 %t4, 2
  ret { ptr, i64, i64 } %t10
}

define internal { ptr, i64, i64 } @arr__pruv_entry(ptr noalias nocapture readonly %p.a.a, i64 %p.a.n) {
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
  %t3 = call { ptr, i64, i64 } @arr__pruv_body(ptr %p.a.a, i64 %p.a.n)
  ret { ptr, i64, i64 } %t3
}

define void @pruv_abi_arr(ptr %a.a.a, i64 %a.a.n, ptr %out) {
entry:
  %r = call { ptr, i64, i64 } @arr__pruv_entry(ptr %a.a.a, i64 %a.a.n)
  %x0r = extractvalue { ptr, i64, i64 } %r, 0
  %x1 = extractvalue { ptr, i64, i64 } %r, 1
  %x2 = extractvalue { ptr, i64, i64 } %r, 2
  %o0 = getelementptr inbounds { ptr, i64 }, ptr %out, i32 0, i32 0
  %o1 = getelementptr inbounds { ptr, i64 }, ptr %out, i32 0, i32 1
  store i64 %x1, ptr %o1
  %x3 = icmp eq i64 %x2, %x1
  br i1 %x3, label %shr.same, label %shr.do
shr.do:
  %x0 = call ptr @pruv_shrink_v2(ptr %x0r, i64 %x2, i64 %x1, i64 8, i64 8)
  store ptr %x0, ptr %o0
  ret void
shr.same:
  store ptr %x0r, ptr %o0
  ret void
}
