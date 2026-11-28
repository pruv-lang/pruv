target datalayout = "e-m:w-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-pc-windows-msvc"


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

define internal i64 @unwrap_or__pruv_body({ i1, i64 } %p.o, i64 %p.d) alwaysinline {
entry:
  br label %start
start:
  %t1 = extractvalue { i1, i64 } %p.o, 0
  %t2 = extractvalue { i1, i64 } %p.o, 1
  br i1 %t1, label %armT.1, label %armF.2
armT.1:
  ret i64 %t2
armF.2:
  ret i64 %p.d
}

define internal i64 @unwrap_or__pruv_entry({ i1, i64 } %p.o, i64 %p.d) {
entry:
  br label %start
start:
  %t1 = call i64 @unwrap_or__pruv_body({ i1, i64 } %p.o, i64 %p.d)
  ret i64 %t1
}

define i64 @pruv_abi_unwrap_or(i64 %a.o.t, i64 %a.o.v, i64 %a.d) {
entry:
  %c.o.t = icmp ne i64 %a.o.t, 0
  %c.o.g1 = insertvalue { i1, i64 } undef, i1 %c.o.t, 0
  %c.o.g = insertvalue { i1, i64 } %c.o.g1, i64 %a.o.v, 1
  %r = call i64 @unwrap_or__pruv_entry({ i1, i64 } %c.o.g, i64 %a.d)
  ret i64 %r
}

define internal i64 @res_code__pruv_body({ i1, i64 } %p.r) alwaysinline {
entry:
  br label %start
start:
  %t1 = extractvalue { i1, i64 } %p.r, 0
  %t2 = extractvalue { i1, i64 } %p.r, 1
  br i1 %t1, label %armT.1, label %armF.2
armT.1:
  ret i64 %t2
armF.2:
  ret i64 %t2
}

define internal i64 @res_code__pruv_entry({ i1, i64 } %p.r) {
entry:
  br label %start
start:
  %t1 = call i64 @res_code__pruv_body({ i1, i64 } %p.r)
  ret i64 %t1
}

define i64 @pruv_abi_res_code(i64 %a.r.t, i64 %a.r.v) {
entry:
  %c.r.t = icmp ne i64 %a.r.t, 0
  %c.r.g1 = insertvalue { i1, i64 } undef, i1 %c.r.t, 0
  %c.r.g = insertvalue { i1, i64 } %c.r.g1, i64 %a.r.v, 1
  %r = call i64 @res_code__pruv_entry({ i1, i64 } %c.r.g)
  ret i64 %r
}
