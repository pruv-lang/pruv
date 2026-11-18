#[allow(dead_code)]
#[cold]
#[inline(never)]
fn pruv_ud(what: &str) -> ! { panic!("pruv: requires undefined: {}", what) }
#[allow(dead_code)]
#[inline(always)]
fn pruv_cidx(a: &[i64], i: i128) -> i128 { if i >= 0 && i < (a.len() as i128) { a[i as usize] as i128 } else { pruv_ud("array index out of range") } }
#[allow(dead_code)]
#[inline(always)]
fn pruv_cadd(a: i128, b: i128) -> i128 { a.checked_add(b).unwrap_or_else(|| pruv_ud("i128 overflow in +")) }
#[allow(dead_code)]
#[inline(always)]
fn pruv_csub(a: i128, b: i128) -> i128 { a.checked_sub(b).unwrap_or_else(|| pruv_ud("i128 overflow in -")) }
#[allow(dead_code)]
#[inline(always)]
fn pruv_cmul(a: i128, b: i128) -> i128 { a.checked_mul(b).unwrap_or_else(|| pruv_ud("i128 overflow in *")) }
#[allow(dead_code)]
#[inline(always)]
fn pruv_cneg(a: i128) -> i128 { a.checked_neg().unwrap_or_else(|| pruv_ud("i128 overflow in unary -")) }
#[allow(dead_code)]
#[inline(always)]
fn pruv_cdiv(a: i128, b: i128) -> i128 { if b == 0 { pruv_ud("division by zero in /") } a.checked_div_euclid(b).unwrap_or_else(|| pruv_ud("i128 overflow in /")) }
#[allow(dead_code)]
#[inline(always)]
fn pruv_crem(a: i128, b: i128) -> i128 { if b == 0 { pruv_ud("division by zero in %") } a.checked_rem_euclid(b).unwrap_or_else(|| pruv_ud("i128 overflow in %")) }

#[allow(unused_mut, unused_variables, unused_parens, non_snake_case, non_camel_case_types, dead_code, unreachable_code, unreachable_patterns, unconditional_panic, arithmetic_overflow)]
#[inline(always)]
fn note_pairs__pruv_body(n0: i64, has: i64, pre: &[i64], keys: &[i64]) -> ::std::vec::Vec<i64> {
    let m: i64 = (keys.len() as i64);
    let p: i64 = (pre.len() as i64);
    let mut on: i64 = has;
    let mut w: i64 = 0i64;
    let mut j: i64 = 0i64;
    while (j < m) {
        if (((unsafe { *keys.get_unchecked((j) as usize) }) > 0i64) && (on == 0i64)) {
            on = 1i64;
            w = j;
        }
        j = (j + 1i64);
    }
    if (on == 0i64) {
        let none: ::std::vec::Vec<i64> = { let pruv_n: i64 = 1i64; let pruv_v: i64 = 0i64; vec![pruv_v; pruv_n as usize] };
        return none;
    }
    let mut base: i64 = n0;
    if ((p > n0) || (m == 0i64)) {
        base = p;
    }
    let mut out: ::std::vec::Vec<i64> = { let pruv_n: i64 = ((1i64 + base) + m); let pruv_v: i64 = 0i64; vec![pruv_v; pruv_n as usize] };
    { let pruv_i: i64 = 0i64; let pruv_x: i64 = 1i64; unsafe { *out.get_unchecked_mut(pruv_i as usize) = pruv_x; } }
    let mut i: i64 = 0i64;
    while (i < p) {
        { let pruv_i: i64 = (1i64 + i); let pruv_x: i64 = (unsafe { *pre.get_unchecked((i) as usize) }); unsafe { *out.get_unchecked_mut(pruv_i as usize) = pruv_x; } }
        i = (i + 1i64);
    }
    let mut k: i64 = 0i64;
    while (k < m) {
        { let pruv_i: i64 = ((1i64 + base) + k); let pruv_x: i64 = (unsafe { *keys.get_unchecked((k) as usize) }); unsafe { *out.get_unchecked_mut(pruv_i as usize) = pruv_x; } }
        k = (k + 1i64);
    }
    return out;
}

#[allow(unused_mut, unused_variables, unused_parens, non_snake_case, non_camel_case_types, dead_code, unreachable_code, unreachable_patterns, unconditional_panic, arithmetic_overflow)]
#[inline(always)]
pub fn note_pairs(n0: i64, has: i64, pre: &[i64], keys: &[i64]) -> ::std::vec::Vec<i64> {
    #[cold] #[inline(never)] fn pruv_len_slow(pre: usize, keys: usize) -> ! { if (pre as u64) > (1u64 << 40) { panic!("pruv: invalid representation: {} (array length)", pre) } if (keys as u64) > (1u64 << 40) { panic!("pruv: invalid representation: {} (array length)", keys) } unreachable!() }
    if ((pre.len() as u64) > (1u64 << 40)) | ((keys.len() as u64) > (1u64 << 40)) { pruv_len_slow(pre.len(), keys.len()) }
    let mut pruv_budget: i64 = 1_073_741_824;
    if !(((0i128 <= (n0 as i128)) && ((n0 as i128) <= 10000000i128))) { panic!("{}", "pruv: requires violated: crate::note_pairs: ((0 <= n0) && (n0 <= 10000000))"); }
    if !((((has as i128) == 0i128) || ((has as i128) == 1i128))) { panic!("{}", "pruv: requires violated: crate::note_pairs: ((has == 0) || (has == 1))"); }
    if !(((pre.len() as i128) <= 10000000i128)) { panic!("{}", "pruv: requires violated: crate::note_pairs: (len(pre) <= 10000000)"); }
    if !((((has as i128) == 1i128) || ((pre.len() as i128) == 0i128))) { panic!("{}", "pruv: requires violated: crate::note_pairs: ((has == 1) || (len(pre) == 0))"); }
    if !(((keys.len() as i128) <= 10000000i128)) { panic!("{}", "pruv: requires violated: crate::note_pairs: (len(keys) <= 10000000)"); }
    if !({ let pruv_lo1: i128 = 0i128; let pruv_hi1: i128 = (pre.len() as i128); let mut pruv_k1: i128 = pruv_lo1; let mut pruv_r1 = true; if !(pruv_lo1 >= pruv_hi1 || (pruv_lo1 >= -9223372036854775808i128 && pruv_hi1 <= 9223372036854775807i128 && pruv_hi1 - pruv_lo1 <= (pruv_budget as i128) && pruv_lo1 >= 0i128 && pruv_hi1 <= (pre.len() as i128) && { let mut pruv_a1 = true; let mut pruv_j1: i64 = pruv_lo1 as i64; let pruv_e1: i64 = pruv_hi1 as i64; while pruv_j1 < pruv_e1 { let i: i128 = pruv_j1 as i128; pruv_a1 &= ((0i128 <= ((unsafe { *pre.get_unchecked(pruv_j1 as usize) }) as i128)) & (((unsafe { *pre.get_unchecked(pruv_j1 as usize) }) as i128) <= 1000000000i128)); pruv_j1 += 1; } pruv_a1 })) { while pruv_k1 < pruv_hi1 { pruv_budget -= 1; if pruv_budget < 0 { pruv_ud("contract iteration budget exceeded"); } let i: i128 = pruv_k1; if !(((0i128 <= pruv_cidx(pre, i)) && (pruv_cidx(pre, i) <= 1000000000i128))) { pruv_r1 = false; break; } pruv_k1 += 1; } } else if pruv_hi1 > pruv_lo1 { let pruv_b = &mut pruv_budget; *pruv_b -= (pruv_hi1 - pruv_lo1) as i64; } pruv_r1 }) { panic!("{}", "pruv: requires violated: crate::note_pairs: forall i in 0..len(pre): ((0 <= pre[i]) && (pre[i] <= 1000000000))"); }
    if !({ let pruv_lo2: i128 = 0i128; let pruv_hi2: i128 = (keys.len() as i128); let mut pruv_k2: i128 = pruv_lo2; let mut pruv_r2 = true; if !(pruv_lo2 >= pruv_hi2 || (pruv_lo2 >= -9223372036854775808i128 && pruv_hi2 <= 9223372036854775807i128 && pruv_hi2 - pruv_lo2 <= (pruv_budget as i128) && pruv_lo2 >= 0i128 && pruv_hi2 <= (keys.len() as i128) && { let mut pruv_a2 = true; let mut pruv_j2: i64 = pruv_lo2 as i64; let pruv_e2: i64 = pruv_hi2 as i64; while pruv_j2 < pruv_e2 { let j: i128 = pruv_j2 as i128; pruv_a2 &= ((0i128 <= ((unsafe { *keys.get_unchecked(pruv_j2 as usize) }) as i128)) & (((unsafe { *keys.get_unchecked(pruv_j2 as usize) }) as i128) <= 1000000000i128)); pruv_j2 += 1; } pruv_a2 })) { while pruv_k2 < pruv_hi2 { pruv_budget -= 1; if pruv_budget < 0 { pruv_ud("contract iteration budget exceeded"); } let j: i128 = pruv_k2; if !(((0i128 <= pruv_cidx(keys, j)) && (pruv_cidx(keys, j) <= 1000000000i128))) { pruv_r2 = false; break; } pruv_k2 += 1; } } else if pruv_hi2 > pruv_lo2 { let pruv_b = &mut pruv_budget; *pruv_b -= (pruv_hi2 - pruv_lo2) as i64; } pruv_r2 }) { panic!("{}", "pruv: requires violated: crate::note_pairs: forall j in 0..len(keys): ((0 <= keys[j]) && (keys[j] <= 1000000000))"); }
    note_pairs__pruv_body(n0, has, pre, keys)
}

