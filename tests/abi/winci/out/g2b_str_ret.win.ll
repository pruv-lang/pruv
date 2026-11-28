target datalayout = "e-m:w-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-pc-windows-msvc"

@.str.0 = private unnamed_addr constant [22 x i8] c"division by zero in /\00"
@.str.1 = private unnamed_addr constant [19 x i8] c"i128 overflow in /\00"
@.str.2 = private unnamed_addr constant [22 x i8] c"division by zero in %\00"
@.str.3 = private unnamed_addr constant [19 x i8] c"i128 overflow in %\00"
@.str.4 = private unnamed_addr constant [4 x i8] c"cut\00"
@.str.5 = private unnamed_addr constant [28 x i8] c"((0 <= i) && (i <= len(s)))\00"
@.str.6 = private unnamed_addr constant [23 x i8] c"is_char_boundary(s, i)\00"
@.str.7 = private unnamed_addr constant [11 x i8] c"str length\00"

declare noalias nonnull ptr @pruv_raw_alloc_v3(i64, i64, i64) nounwind memory(inaccessiblemem: readwrite)
declare noalias nonnull ptr @pruv_raw_alloc_zeroed_v3(i64, i64, i64) nounwind memory(inaccessiblemem: readwrite)
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
declare i32 @pruv_str_boundary(ptr nocapture readonly, i64, i64) nounwind willreturn memory(argmem: read)
declare noalias ptr @pruv_substr(ptr nocapture readonly, i64, i64, i64) nounwind
declare void @pruv_check_utf8(ptr nocapture readonly, i64) nounwind
declare void @pruv_panic_repr(i64, ptr) noreturn cold nounwind
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

define internal { ptr, i64, i64 } @cut__pruv_body(ptr noalias nocapture readonly %p.s.a, i64 %p.s.n, i64 %p.i) alwaysinline {
entry:
  %l0.c.a = alloca ptr
  %l0.c.b = alloca i64
  %l0.c.c = alloca i64
  %l1.d.a = alloca ptr
  %l1.d.b = alloca i64
  %l1.d.c = alloca i64
  br label %start
start:
  %t1 = call ptr @pruv_copy_v2(ptr %p.s.a, i64 %p.s.n, i64 1, i64 1)
  store ptr %t1, ptr %l0.c.a
  store i64 %p.s.n, ptr %l0.c.b
  store i64 %p.s.n, ptr %l0.c.c
  %t2 = load ptr, ptr %l0.c.a
  %t3 = load i64, ptr %l0.c.b
  %t4 = load i64, ptr %l0.c.c
  %t5 = sub i64 %p.i, 0
  %t6 = getelementptr inbounds i8, ptr %t2, i64 0
  %t7 = call ptr @pruv_copy_v2(ptr %t6, i64 %t5, i64 1, i64 1)
  store ptr %t7, ptr %l1.d.a
  store i64 %t5, ptr %l1.d.b
  store i64 %t5, ptr %l1.d.c
  %t8 = load ptr, ptr %l1.d.a
  %t9 = load i64, ptr %l1.d.b
  %t10 = load i64, ptr %l1.d.c
  store ptr null, ptr %l1.d.a
  store i64 0, ptr %l1.d.b
  store i64 0, ptr %l1.d.c
  %t11 = load ptr, ptr %l1.d.a
  %t12 = load i64, ptr %l1.d.b
  %t13 = load i64, ptr %l1.d.c
  call void @pruv_free_v2_nn(ptr %t11, i64 %t13, i64 1, i64 1)
  %t14 = load ptr, ptr %l0.c.a
  %t15 = load i64, ptr %l0.c.b
  %t16 = load i64, ptr %l0.c.c
  call void @pruv_free_v2_nn(ptr %t14, i64 %t16, i64 1, i64 1)
  %t17 = insertvalue { ptr, i64, i64 } undef, ptr %t8, 0
  %t18 = insertvalue { ptr, i64, i64 } %t17, i64 %t9, 1
  %t19 = insertvalue { ptr, i64, i64 } %t18, i64 %t10, 2
  ret { ptr, i64, i64 } %t19
}

define internal { ptr, i64, i64 } @cut__pruv_entry(ptr noalias nocapture readonly %p.s.a, i64 %p.s.n, i64 %p.i) {
entry:
  %req.budget = alloca i64
  br label %start
start:
  %t1 = sext i64 %p.i to i128
  store i64 1073741824, ptr %req.budget
  %t2 = icmp sle i128 0, %t1
  br i1 %t2, label %rhs.1, label %endsc.2
rhs.1:
  %t3 = zext i64 %p.s.n to i128
  %t4 = icmp sle i128 %t1, %t3
  br label %endsc.2
endsc.2:
  %t5 = phi i1 [ false, %start ], [ %t4, %rhs.1 ]
  br i1 %t5, label %ok.3, label %fail.4
fail.4:
  call void @pruv_panic_requires(ptr @.str.4, ptr @.str.5)
  unreachable
ok.3:
  %t6 = zext i64 %p.s.n to i128
  %t7 = icmp ule i128 %t1, %t6
  %t8 = trunc i128 %t1 to i64
  %t9 = select i1 %t7, i64 %t8, i64 -1
  %t10 = call i32 @pruv_str_boundary(ptr %p.s.a, i64 %p.s.n, i64 %t9)
  %t11 = icmp ne i32 %t10, 0
  br i1 %t11, label %ok.5, label %fail.6
fail.6:
  call void @pruv_panic_requires(ptr @.str.4, ptr @.str.6)
  unreachable
ok.5:
  %t12 = call { ptr, i64, i64 } @cut__pruv_body(ptr %p.s.a, i64 %p.s.n, i64 %p.i)
  ret { ptr, i64, i64 } %t12
}

define void @pruv_abi_cut(ptr %a.s.a, i64 %a.s.n, i64 %a.i, ptr %out) {
entry:
  %ln.p0 = icmp ugt i64 %a.s.n, 1099511627776
  br i1 %ln.p0, label %len.bad.0, label %len.ok.0
len.bad.0:
  call void @pruv_panic_repr(i64 %a.s.n, ptr @.str.7)
  unreachable
len.ok.0:
  call void @pruv_check_utf8(ptr %a.s.a, i64 %a.s.n)
  %r = call { ptr, i64, i64 } @cut__pruv_entry(ptr %a.s.a, i64 %a.s.n, i64 %a.i)
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

define void @pruv_abi_cut__typed(ptr %a.s.a, i64 %a.s.n, i64 %a.i, ptr %out) {
entry:
  %ln.p0 = icmp ugt i64 %a.s.n, 1099511627776
  br i1 %ln.p0, label %len.bad.0, label %len.ok.0
len.bad.0:
  call void @pruv_panic_repr(i64 %a.s.n, ptr @.str.7)
  unreachable
len.ok.0:
  %r = call { ptr, i64, i64 } @cut__pruv_entry(ptr %a.s.a, i64 %a.s.n, i64 %a.i)
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
