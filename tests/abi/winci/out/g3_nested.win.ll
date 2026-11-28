target datalayout = "e-m:w-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-pc-windows-msvc"


%pruv_t_pruv_g3_t_4_Pair_2_b_ob = type { i1, { i1, i1 } }
%pruv_t_pruv_g3_t_4_Pair_2_ob_b = type { { i1, i1 }, i1 }
%pruv_t_pruv_g3_t_4_Pair_2_ob_ri = type { { i1, i1 }, { i1, i64 } }
%pruv_c_pruv_g3_t_4_Pair_2_b_ob = type { i1, { i1, i1 } }
%pruv_c_pruv_g3_t_4_Pair_2_ob_b = type { { i1, i1 }, i1 }
%pruv_c_pruv_g3_t_4_Pair_2_ob_ri = type { { i1, i1 }, { i1, i128 } }

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

define internal %pruv_t_pruv_g3_t_4_Pair_2_ob_ri @nest_id__pruv_body(%pruv_t_pruv_g3_t_4_Pair_2_ob_ri %p.p) alwaysinline {
entry:
  br label %start
start:
  ret %pruv_t_pruv_g3_t_4_Pair_2_ob_ri %p.p
}

define internal %pruv_t_pruv_g3_t_4_Pair_2_ob_ri @nest_id__pruv_entry(%pruv_t_pruv_g3_t_4_Pair_2_ob_ri %p.p) {
entry:
  br label %start
start:
  %t1 = call %pruv_t_pruv_g3_t_4_Pair_2_ob_ri @nest_id__pruv_body(%pruv_t_pruv_g3_t_4_Pair_2_ob_ri %p.p)
  ret %pruv_t_pruv_g3_t_4_Pair_2_ob_ri %t1
}

define void @pruv_abi_nest_id(ptr %a.p, ptr %out) {
entry:
  %w.1 = getelementptr inbounds i64, ptr %a.p, i64 0
  %w.2 = load i64, ptr %w.1
  %w.3 = icmp ne i64 %w.2, 0
  %w.4 = getelementptr inbounds i64, ptr %a.p, i64 1
  %w.5 = load i64, ptr %w.4
  %w.6 = icmp ne i64 %w.5, 0
  %w.7 = and i1 %w.3, %w.6
  %w.8 = insertvalue { i1, i1 } zeroinitializer, i1 %w.3, 0
  %w.9 = insertvalue { i1, i1 } %w.8, i1 %w.7, 1
  %w.10 = insertvalue %pruv_t_pruv_g3_t_4_Pair_2_ob_ri zeroinitializer, { i1, i1 } %w.9, 0
  %w.11 = getelementptr inbounds i64, ptr %a.p, i64 2
  %w.12 = load i64, ptr %w.11
  %w.13 = icmp ne i64 %w.12, 0
  %w.14 = getelementptr inbounds i64, ptr %a.p, i64 3
  %w.15 = load i64, ptr %w.14
  %w.16 = insertvalue { i1, i64 } undef, i1 %w.13, 0
  %w.17 = insertvalue { i1, i64 } %w.16, i64 %w.15, 1
  %w.18 = insertvalue %pruv_t_pruv_g3_t_4_Pair_2_ob_ri %w.10, { i1, i64 } %w.17, 1
  %r = call %pruv_t_pruv_g3_t_4_Pair_2_ob_ri @nest_id__pruv_entry(%pruv_t_pruv_g3_t_4_Pair_2_ob_ri %w.18)
  %w.19 = extractvalue %pruv_t_pruv_g3_t_4_Pair_2_ob_ri %r, 0
  %w.20 = extractvalue { i1, i1 } %w.19, 0
  %w.21 = zext i1 %w.20 to i64
  %w.22 = getelementptr inbounds i64, ptr %out, i64 0
  store i64 %w.21, ptr %w.22
  %w.23 = extractvalue { i1, i1 } %w.19, 1
  %w.24 = and i1 %w.20, %w.23
  %w.25 = zext i1 %w.24 to i64
  %w.26 = getelementptr inbounds i64, ptr %out, i64 1
  store i64 %w.25, ptr %w.26
  %w.27 = extractvalue %pruv_t_pruv_g3_t_4_Pair_2_ob_ri %r, 1
  %w.28 = extractvalue { i1, i64 } %w.27, 0
  %w.29 = extractvalue { i1, i64 } %w.27, 1
  %w.30 = zext i1 %w.28 to i64
  %w.31 = getelementptr inbounds i64, ptr %out, i64 2
  store i64 %w.30, ptr %w.31
  %w.32 = getelementptr inbounds i64, ptr %out, i64 3
  store i64 %w.29, ptr %w.32
  ret void
}

define internal %pruv_t_pruv_g3_t_4_Pair_2_ob_b @nest_pick__pruv_body(%pruv_t_pruv_g3_t_4_Pair_2_ob_ri %p.p, %pruv_t_pruv_g3_t_4_Pair_2_b_ob %p.q) alwaysinline {
entry:
  br label %start
start:
  %t1 = extractvalue %pruv_t_pruv_g3_t_4_Pair_2_b_ob %p.q, 1
  %t2 = extractvalue %pruv_t_pruv_g3_t_4_Pair_2_b_ob %p.q, 0
  %t3 = insertvalue %pruv_t_pruv_g3_t_4_Pair_2_ob_b zeroinitializer, { i1, i1 } %t1, 0
  %t4 = insertvalue %pruv_t_pruv_g3_t_4_Pair_2_ob_b %t3, i1 %t2, 1
  ret %pruv_t_pruv_g3_t_4_Pair_2_ob_b %t4
}

define internal %pruv_t_pruv_g3_t_4_Pair_2_ob_b @nest_pick__pruv_entry(%pruv_t_pruv_g3_t_4_Pair_2_ob_ri %p.p, %pruv_t_pruv_g3_t_4_Pair_2_b_ob %p.q) {
entry:
  br label %start
start:
  %t1 = call %pruv_t_pruv_g3_t_4_Pair_2_ob_b @nest_pick__pruv_body(%pruv_t_pruv_g3_t_4_Pair_2_ob_ri %p.p, %pruv_t_pruv_g3_t_4_Pair_2_b_ob %p.q)
  ret %pruv_t_pruv_g3_t_4_Pair_2_ob_b %t1
}

define void @pruv_abi_nest_pick(ptr %a.p, ptr %a.q, ptr %out) {
entry:
  %w.1 = getelementptr inbounds i64, ptr %a.p, i64 0
  %w.2 = load i64, ptr %w.1
  %w.3 = icmp ne i64 %w.2, 0
  %w.4 = getelementptr inbounds i64, ptr %a.p, i64 1
  %w.5 = load i64, ptr %w.4
  %w.6 = icmp ne i64 %w.5, 0
  %w.7 = and i1 %w.3, %w.6
  %w.8 = insertvalue { i1, i1 } zeroinitializer, i1 %w.3, 0
  %w.9 = insertvalue { i1, i1 } %w.8, i1 %w.7, 1
  %w.10 = insertvalue %pruv_t_pruv_g3_t_4_Pair_2_ob_ri zeroinitializer, { i1, i1 } %w.9, 0
  %w.11 = getelementptr inbounds i64, ptr %a.p, i64 2
  %w.12 = load i64, ptr %w.11
  %w.13 = icmp ne i64 %w.12, 0
  %w.14 = getelementptr inbounds i64, ptr %a.p, i64 3
  %w.15 = load i64, ptr %w.14
  %w.16 = insertvalue { i1, i64 } undef, i1 %w.13, 0
  %w.17 = insertvalue { i1, i64 } %w.16, i64 %w.15, 1
  %w.18 = insertvalue %pruv_t_pruv_g3_t_4_Pair_2_ob_ri %w.10, { i1, i64 } %w.17, 1
  %w.19 = getelementptr inbounds i64, ptr %a.q, i64 0
  %w.20 = load i64, ptr %w.19
  %w.21 = icmp ne i64 %w.20, 0
  %w.22 = insertvalue %pruv_t_pruv_g3_t_4_Pair_2_b_ob zeroinitializer, i1 %w.21, 0
  %w.23 = getelementptr inbounds i64, ptr %a.q, i64 1
  %w.24 = load i64, ptr %w.23
  %w.25 = icmp ne i64 %w.24, 0
  %w.26 = getelementptr inbounds i64, ptr %a.q, i64 2
  %w.27 = load i64, ptr %w.26
  %w.28 = icmp ne i64 %w.27, 0
  %w.29 = and i1 %w.25, %w.28
  %w.30 = insertvalue { i1, i1 } zeroinitializer, i1 %w.25, 0
  %w.31 = insertvalue { i1, i1 } %w.30, i1 %w.29, 1
  %w.32 = insertvalue %pruv_t_pruv_g3_t_4_Pair_2_b_ob %w.22, { i1, i1 } %w.31, 1
  %r = call %pruv_t_pruv_g3_t_4_Pair_2_ob_b @nest_pick__pruv_entry(%pruv_t_pruv_g3_t_4_Pair_2_ob_ri %w.18, %pruv_t_pruv_g3_t_4_Pair_2_b_ob %w.32)
  %w.33 = extractvalue %pruv_t_pruv_g3_t_4_Pair_2_ob_b %r, 0
  %w.34 = extractvalue { i1, i1 } %w.33, 0
  %w.35 = zext i1 %w.34 to i64
  %w.36 = getelementptr inbounds i64, ptr %out, i64 0
  store i64 %w.35, ptr %w.36
  %w.37 = extractvalue { i1, i1 } %w.33, 1
  %w.38 = and i1 %w.34, %w.37
  %w.39 = zext i1 %w.38 to i64
  %w.40 = getelementptr inbounds i64, ptr %out, i64 1
  store i64 %w.39, ptr %w.40
  %w.41 = extractvalue %pruv_t_pruv_g3_t_4_Pair_2_ob_b %r, 1
  %w.42 = zext i1 %w.41 to i64
  %w.43 = getelementptr inbounds i64, ptr %out, i64 2
  store i64 %w.42, ptr %w.43
  ret void
}
