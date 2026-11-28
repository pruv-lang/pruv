target datalayout = "e-m:w-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-pc-windows-msvc"

@.str.0 = private unnamed_addr constant [22 x i8] c"division by zero in /\00"
@.str.1 = private unnamed_addr constant [19 x i8] c"i128 overflow in /\00"
@.str.2 = private unnamed_addr constant [22 x i8] c"division by zero in %\00"
@.str.3 = private unnamed_addr constant [19 x i8] c"i128 overflow in %\00"
@.str.4 = private unnamed_addr constant [16 x i8] c"crate::positive\00"
@.str.5 = private unnamed_addr constant [8 x i8] c"(x > 0)\00"
@.str.6 = private unnamed_addr constant [13 x i8] c"array length\00"
@.str.7 = private unnamed_addr constant [25 x i8] c"array index out of range\00"
@.str.8 = private unnamed_addr constant [22 x i8] c"crate::checked_length\00"
@.str.9 = private unnamed_addr constant [33 x i8] c"((len(sid) > 0) && (sid[0] > 0))\00"
@.str.10 = private unnamed_addr constant [41 x i8] c"inout argument overlaps another argument\00"

%pruv_t_Mixed = type { i8, i16, i64 }
%pruv_c_Mixed = type { i128, i128, i128 }

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
declare void @pruv_panic_repr(i64, ptr) noreturn cold nounwind
declare ptr @pruv_shrink_v2(ptr, i64, i64, i64, i64) nounwind
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
declare ptr @pruv_regrow_v2(ptr, i64, i64, i64, i64) nounwind

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

define internal i8 @id_u8__pruv_body(i8 %p.x) alwaysinline {
entry:
  br label %start
start:
  ret i8 %p.x
}

define i8 @pruv_abi_id_u8(i8 %a.x) {
entry:
  %r = call i8 @id_u8__pruv_body(i8 %a.x)
  ret i8 %r
}

define internal i16 @id_u16__pruv_body(i16 %p.x) alwaysinline {
entry:
  br label %start
start:
  ret i16 %p.x
}

define i16 @pruv_abi_id_u16(i16 %a.x) {
entry:
  %r = call i16 @id_u16__pruv_body(i16 %a.x)
  ret i16 %r
}

define internal i32 @id_u32__pruv_body(i32 %p.x) alwaysinline {
entry:
  br label %start
start:
  ret i32 %p.x
}

define i32 @pruv_abi_id_u32(i32 %a.x) {
entry:
  %r = call i32 @id_u32__pruv_body(i32 %a.x)
  ret i32 %r
}

define internal i64 @id_u64__pruv_body(i64 %p.x) alwaysinline {
entry:
  br label %start
start:
  ret i64 %p.x
}

define i64 @pruv_abi_id_u64(i64 %a.x) {
entry:
  %r = call i64 @id_u64__pruv_body(i64 %a.x)
  ret i64 %r
}

define internal i64 @id_usize__pruv_body(i64 %p.x) alwaysinline {
entry:
  br label %start
start:
  ret i64 %p.x
}

define i64 @pruv_abi_id_usize(i64 %a.x) {
entry:
  %r = call i64 @id_usize__pruv_body(i64 %a.x)
  ret i64 %r
}

define internal i8 @id_i8__pruv_body(i8 %p.x) alwaysinline {
entry:
  br label %start
start:
  ret i8 %p.x
}

define i8 @pruv_abi_id_i8(i8 %a.x) {
entry:
  %r = call i8 @id_i8__pruv_body(i8 %a.x)
  ret i8 %r
}

define internal i16 @id_i16__pruv_body(i16 %p.x) alwaysinline {
entry:
  br label %start
start:
  ret i16 %p.x
}

define i16 @pruv_abi_id_i16(i16 %a.x) {
entry:
  %r = call i16 @id_i16__pruv_body(i16 %a.x)
  ret i16 %r
}

define internal i32 @id_i32__pruv_body(i32 %p.x) alwaysinline {
entry:
  br label %start
start:
  ret i32 %p.x
}

define i32 @pruv_abi_id_i32(i32 %a.x) {
entry:
  %r = call i32 @id_i32__pruv_body(i32 %a.x)
  ret i32 %r
}

define internal i64 @id_isize__pruv_body(i64 %p.x) alwaysinline {
entry:
  br label %start
start:
  ret i64 %p.x
}

define i64 @pruv_abi_id_isize(i64 %a.x) {
entry:
  %r = call i64 @id_isize__pruv_body(i64 %a.x)
  ret i64 %r
}

define internal i64 @positive__pruv_body(i64 %p.x) alwaysinline {
entry:
  br label %start
start:
  ret i64 %p.x
}

define internal i64 @positive__pruv_entry(i64 %p.x) {
entry:
  %req.budget = alloca i64
  br label %start
start:
  %t1 = zext i64 %p.x to i128
  store i64 1073741824, ptr %req.budget
  %t2 = icmp sgt i128 %t1, 0
  br i1 %t2, label %ok.1, label %fail.2
fail.2:
  call void @pruv_panic_requires(ptr @.str.4, ptr @.str.5)
  unreachable
ok.1:
  %t3 = call i64 @positive__pruv_body(i64 %p.x)
  ret i64 %t3
}

define i64 @pruv_abi_positive(i64 %a.x) {
entry:
  %r = call i64 @positive__pruv_entry(i64 %a.x)
  ret i64 %r
}

define internal %pruv_t_Mixed @round_trip__pruv_body(%pruv_t_Mixed %p.p) alwaysinline {
entry:
  br label %start
start:
  ret %pruv_t_Mixed %p.p
}

define void @pruv_abi_round_trip(ptr %a.p, ptr %out) {
entry:
  %w.1 = getelementptr inbounds i64, ptr %a.p, i64 0
  %w.2 = load i64, ptr %w.1
  %w.3 = trunc i64 %w.2 to i8
  %w.4 = insertvalue %pruv_t_Mixed zeroinitializer, i8 %w.3, 0
  %w.5 = getelementptr inbounds i64, ptr %a.p, i64 1
  %w.6 = load i64, ptr %w.5
  %w.7 = trunc i64 %w.6 to i16
  %w.8 = insertvalue %pruv_t_Mixed %w.4, i16 %w.7, 1
  %w.9 = getelementptr inbounds i64, ptr %a.p, i64 2
  %w.10 = load i64, ptr %w.9
  %w.11 = insertvalue %pruv_t_Mixed %w.8, i64 %w.10, 2
  %r = call %pruv_t_Mixed @round_trip__pruv_body(%pruv_t_Mixed %w.11)
  %w.12 = extractvalue %pruv_t_Mixed %r, 0
  %w.13 = zext i8 %w.12 to i64
  %w.14 = getelementptr inbounds i64, ptr %out, i64 0
  store i64 %w.13, ptr %w.14
  %w.15 = extractvalue %pruv_t_Mixed %r, 1
  %w.16 = sext i16 %w.15 to i64
  %w.17 = getelementptr inbounds i64, ptr %out, i64 1
  store i64 %w.16, ptr %w.17
  %w.18 = extractvalue %pruv_t_Mixed %r, 2
  %w.19 = getelementptr inbounds i64, ptr %out, i64 2
  store i64 %w.18, ptr %w.19
  ret void
}

define internal i64 @length__pruv_body(ptr noalias nocapture readonly %p.sid.a, i64 %p.sid.n) alwaysinline {
entry:
  br label %start
start:
  ret i64 %p.sid.n
}

define i64 @pruv_abi_length(ptr %a.sid.a, i64 %a.sid.n) {
entry:
  %ln.p0 = icmp ugt i64 %a.sid.n, 1099511627776
  br i1 %ln.p0, label %len.bad.0, label %len.ok.0
len.bad.0:
  call void @pruv_panic_repr(i64 %a.sid.n, ptr @.str.6)
  unreachable
len.ok.0:
  %r = call i64 @length__pruv_body(ptr %a.sid.a, i64 %a.sid.n)
  ret i64 %r
}

define internal { ptr, i64, i64 } @units__pruv_body(i8 %p.a, i16 %p.b, i64 %p.c, ptr noalias nocapture readonly %p.sid.a, i64 %p.sid.n) alwaysinline {
entry:
  %l0.result_buf.a = alloca { ptr, i64, i64 }
  br label %start
start:
  %t1 = call ptr @pruv_copy_v2(ptr %p.sid.a, i64 %p.sid.n, i64 2, i64 2)
  %t2 = getelementptr inbounds { ptr, i64, i64 }, ptr %l0.result_buf.a, i32 0, i32 1
  %t3 = getelementptr inbounds { ptr, i64, i64 }, ptr %l0.result_buf.a, i32 0, i32 2
  store ptr %t1, ptr %l0.result_buf.a
  store i64 %p.sid.n, ptr %t2
  store i64 %p.sid.n, ptr %t3
  %t4 = load ptr, ptr %l0.result_buf.a
  %t5 = load i64, ptr %t2
  %t6 = load i64, ptr %t3
  %t7 = add i64 %t5, 4
  %t8 = icmp ult i64 %t6, %t7
  br i1 %t8, label %regrow.1, label %grown.2
regrow.1:
  %t9 = call ptr @pruv_regrow_v2(ptr %t4, i64 %t6, i64 %t7, i64 2, i64 2)
  store ptr %t9, ptr %l0.result_buf.a
  store i64 %t7, ptr %t3
  br label %grown.2
grown.2:
  %t10 = load ptr, ptr %l0.result_buf.a
  %t11 = load i64, ptr %t2
  %t12 = load i64, ptr %t3
  store ptr null, ptr %l0.result_buf.a
  store i64 0, ptr %t2
  store i64 0, ptr %t3
  %t13 = load ptr, ptr %l0.result_buf.a
  %t14 = load i64, ptr %t2
  %t15 = load i64, ptr %t3
  call void @pruv_free_v2_nn(ptr %t13, i64 %t15, i64 2, i64 2)
  %t16 = insertvalue { ptr, i64, i64 } undef, ptr %t10, 0
  %t17 = insertvalue { ptr, i64, i64 } %t16, i64 %t11, 1
  %t18 = insertvalue { ptr, i64, i64 } %t17, i64 %t12, 2
  ret { ptr, i64, i64 } %t18
}

define void @pruv_abi_units(i8 %a.a, i16 %a.b, i64 %a.c, ptr %a.sid.a, i64 %a.sid.n, ptr %out) {
entry:
  %ln.p0 = icmp ugt i64 %a.sid.n, 1099511627776
  br i1 %ln.p0, label %len.bad.0, label %len.ok.0
len.bad.0:
  call void @pruv_panic_repr(i64 %a.sid.n, ptr @.str.6)
  unreachable
len.ok.0:
  %r = call { ptr, i64, i64 } @units__pruv_body(i8 %a.a, i16 %a.b, i64 %a.c, ptr %a.sid.a, i64 %a.sid.n)
  %x0r = extractvalue { ptr, i64, i64 } %r, 0
  %x1 = extractvalue { ptr, i64, i64 } %r, 1
  %x2 = extractvalue { ptr, i64, i64 } %r, 2
  %o0 = getelementptr inbounds { ptr, i64 }, ptr %out, i32 0, i32 0
  %o1 = getelementptr inbounds { ptr, i64 }, ptr %out, i32 0, i32 1
  store i64 %x1, ptr %o1
  %x3 = icmp eq i64 %x2, %x1
  br i1 %x3, label %shr.same, label %shr.do
shr.do:
  %x0 = call ptr @pruv_shrink_v2(ptr %x0r, i64 %x2, i64 %x1, i64 2, i64 2)
  store ptr %x0, ptr %o0
  ret void
shr.same:
  store ptr %x0r, ptr %o0
  ret void
}

define internal i64 @checked_length__pruv_body(ptr noalias nocapture %p.bytes.a, i64 %p.bytes.n, ptr noalias nocapture readonly %p.sid.a, i64 %p.sid.n) alwaysinline {
entry:
  br label %start
start:
  call void @pruv_print_i64(i64 123)
  ret i64 %p.bytes.n
}

define internal i64 @checked_length__pruv_entry(ptr noalias nocapture %p.bytes.a, i64 %p.bytes.n, ptr noalias nocapture readonly %p.sid.a, i64 %p.sid.n) {
entry:
  %req.budget = alloca i64
  br label %start
start:
  store i64 1073741824, ptr %req.budget
  %t1 = zext i64 %p.sid.n to i128
  %t2 = icmp sgt i128 %t1, 0
  br i1 %t2, label %rhs.1, label %endsc.2
rhs.1:
  %t3 = zext i64 %p.sid.n to i128
  %t4 = icmp ult i128 0, %t3
  br i1 %t4, label %ok.3, label %fail.4
fail.4:
  call void @pruv_panic_undefined(ptr @.str.7)
  unreachable
ok.3:
  %t5 = trunc i128 0 to i64
  %t6 = getelementptr inbounds i16, ptr %p.sid.a, i64 %t5
  %t7 = load i16, ptr %t6
  %t8 = zext i16 %t7 to i128
  %t9 = icmp sgt i128 %t8, 0
  br label %endsc.2
endsc.2:
  %t10 = phi i1 [ false, %start ], [ %t9, %ok.3 ]
  br i1 %t10, label %ok.5, label %fail.6
fail.6:
  call void @pruv_panic_requires(ptr @.str.8, ptr @.str.9)
  unreachable
ok.5:
  %t11 = call i64 @checked_length__pruv_body(ptr %p.bytes.a, i64 %p.bytes.n, ptr %p.sid.a, i64 %p.sid.n)
  ret i64 %t11
}

define i64 @pruv_abi_checked_length(ptr %a.bytes.a, i64 %a.bytes.n, ptr %a.sid.a, i64 %a.sid.n) {
entry:
  %ln.or.0 = or i64 %a.bytes.n, %a.sid.n
  %ln.any = icmp ugt i64 %ln.or.0, 1099511627776
  br i1 %ln.any, label %len.slow, label %len.ok
len.slow:
  %ln.p0 = icmp ugt i64 %a.bytes.n, 1099511627776
  br i1 %ln.p0, label %len.bad.0, label %len.ok.0
len.bad.0:
  call void @pruv_panic_repr(i64 %a.bytes.n, ptr @.str.6)
  unreachable
len.ok.0:
  %ln.p1 = icmp ugt i64 %a.sid.n, 1099511627776
  br i1 %ln.p1, label %len.bad.1, label %len.ok.1
len.bad.1:
  call void @pruv_panic_repr(i64 %a.sid.n, ptr @.str.6)
  unreachable
len.ok.1:
  br label %len.ok
len.ok:
  %ov.0 = call i1 @pruv_overlap(ptr %a.bytes.a, i64 %a.bytes.n, i64 1, ptr %a.sid.a, i64 %a.sid.n, i64 2)
  br i1 %ov.0, label %ovl.bad, label %ovl.ok
ovl.bad:
  call void @pruv_panic_requires(ptr @.str.8, ptr @.str.10)
  unreachable
ovl.ok:
  %r = call i64 @checked_length__pruv_entry(ptr %a.bytes.a, i64 %a.bytes.n, ptr %a.sid.a, i64 %a.sid.n)
  ret i64 %r
}
