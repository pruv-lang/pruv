mod pruv_impl {
#[allow(unused_macros)]
macro_rules! assert { ($($t:tt)*) => { core::assert!($($t)*) } }
#[allow(unused_macros)]
macro_rules! eprintln { ($($t:tt)*) => { std::eprintln!($($t)*) } }
#[allow(unused_macros)]
macro_rules! format_args { ($($t:tt)*) => { core::format_args!($($t)*) } }
#[allow(unused_macros)]
macro_rules! matches { ($($t:tt)*) => { core::matches!($($t)*) } }
#[allow(unused_macros)]
macro_rules! panic { ($($t:tt)*) => { core::panic!($($t)*) } }
#[allow(unused_macros)]
macro_rules! println { ($($t:tt)*) => { std::println!($($t)*) } }
#[allow(unused_macros)]
macro_rules! unreachable { ($($t:tt)*) => { core::unreachable!($($t)*) } }
#[allow(unused_macros)]
macro_rules! vec { ($($t:tt)*) => { std::vec!($($t)*) } }
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
fn evict_stop__pruv_body(bytes: &[i64], free: &[i64], limit: i64) -> i64 {
    let n: i64 = (bytes.len() as i64);
    let mut total: i64 = 0i64;
    let mut i: i64 = 0i64;
    while (i < n) {
        total = (total + (unsafe { *bytes.get_unchecked((i) as usize) }));
        i = (i + 1i64);
    }
    let mut left: i64 = total;
    let mut s: i64 = 0i64;
    while (s < n) {
        if (left <= limit) {
            return s;
        }
        left = (left - (unsafe { *free.get_unchecked((s) as usize) }));
        s = (s + 1i64);
    }
    return n;
}

#[allow(unused_mut, unused_variables, unused_parens, non_snake_case, non_camel_case_types, dead_code, unreachable_code, unreachable_patterns, unconditional_panic, arithmetic_overflow)]
#[inline]
pub fn evict_stop(bytes: &[i64], free: &[i64], limit: i64) -> i64 {
    #[cold] #[inline(never)] fn pruv_len_slow(bytes: usize, free: usize) -> ! { if (bytes as u64) > (1u64 << 40) { panic!("pruv: invalid representation: {} (array length)", bytes) } if (free as u64) > (1u64 << 40) { panic!("pruv: invalid representation: {} (array length)", free) } unreachable!() }
    if ((bytes.len() as u64) > (1u64 << 40)) | ((free.len() as u64) > (1u64 << 40)) { pruv_len_slow(bytes.len(), free.len()) }
    let mut pruv_budget: i64 = 1_073_741_824;
    if !(((bytes.len() as i128) <= 10000000i128)) { panic!("{}", "pruv: requires violated: crate::evict_stop: (len(bytes) <= 10000000)"); }
    if !(((free.len() as i128) == (bytes.len() as i128))) { panic!("{}", "pruv: requires violated: crate::evict_stop: (len(free) == len(bytes))"); }
    if !({ let pruv_lo1: i128 = 0i128; let pruv_hi1: i128 = (bytes.len() as i128); let mut pruv_k1: i128 = pruv_lo1; let mut pruv_r1 = true; if !(pruv_lo1 >= pruv_hi1 || (pruv_lo1 >= -9223372036854775808i128 && pruv_hi1 <= 9223372036854775807i128 && pruv_hi1 - pruv_lo1 <= (pruv_budget as i128) && pruv_lo1 >= 0i128 && pruv_hi1 <= (bytes.len() as i128) && { let mut pruv_a1 = true; let mut pruv_j1: i64 = pruv_lo1 as i64; let pruv_e1: i64 = pruv_hi1 as i64; while pruv_j1 < pruv_e1 { let k: i128 = pruv_j1 as i128; pruv_a1 &= ((0i128 <= ((unsafe { *bytes.get_unchecked(pruv_j1 as usize) }) as i128)) & (((unsafe { *bytes.get_unchecked(pruv_j1 as usize) }) as i128) <= 1000000000i128)); pruv_j1 += 1; } pruv_a1 })) { while pruv_k1 < pruv_hi1 { pruv_budget -= 1; if pruv_budget < 0 { pruv_ud("contract iteration budget exceeded"); } let k: i128 = pruv_k1; if !(((0i128 <= pruv_cidx(bytes, k)) && (pruv_cidx(bytes, k) <= 1000000000i128))) { pruv_r1 = false; break; } pruv_k1 += 1; } } else if pruv_hi1 > pruv_lo1 { let pruv_b = &mut pruv_budget; *pruv_b -= (pruv_hi1 - pruv_lo1) as i64; } pruv_r1 }) { panic!("{}", "pruv: requires violated: crate::evict_stop: forall k in 0..len(bytes): ((0 <= bytes[k]) && (bytes[k] <= 1000000000))"); }
    if !({ let pruv_lo2: i128 = 0i128; let pruv_hi2: i128 = (bytes.len() as i128); let mut pruv_k2: i128 = pruv_lo2; let mut pruv_r2 = true; if !(pruv_lo2 >= pruv_hi2 || (pruv_lo2 >= -9223372036854775808i128 && pruv_hi2 <= 9223372036854775807i128 && pruv_hi2 - pruv_lo2 <= (pruv_budget as i128) && pruv_lo2 >= 0i128 && pruv_hi2 <= (free.len() as i128) && pruv_lo2 >= 0i128 && pruv_hi2 <= (bytes.len() as i128) && { let mut pruv_a2 = true; let mut pruv_j2: i64 = pruv_lo2 as i64; let pruv_e2: i64 = pruv_hi2 as i64; while pruv_j2 < pruv_e2 { let k: i128 = pruv_j2 as i128; pruv_a2 &= ((((unsafe { *free.get_unchecked(pruv_j2 as usize) }) as i128) == 0i128) | (((unsafe { *free.get_unchecked(pruv_j2 as usize) }) as i128) == ((unsafe { *bytes.get_unchecked(pruv_j2 as usize) }) as i128))); pruv_j2 += 1; } pruv_a2 })) { while pruv_k2 < pruv_hi2 { pruv_budget -= 1; if pruv_budget < 0 { pruv_ud("contract iteration budget exceeded"); } let k: i128 = pruv_k2; if !(((pruv_cidx(free, k) == 0i128) || (pruv_cidx(free, k) == pruv_cidx(bytes, k)))) { pruv_r2 = false; break; } pruv_k2 += 1; } } else if pruv_hi2 > pruv_lo2 { let pruv_b = &mut pruv_budget; *pruv_b -= (pruv_hi2 - pruv_lo2) as i64; } pruv_r2 }) { panic!("{}", "pruv: requires violated: crate::evict_stop: forall k in 0..len(bytes): ((free[k] == 0) || (free[k] == bytes[k]))"); }
    if !((((-1000000000000000i128) <= (limit as i128)) && ((limit as i128) <= 1000000000000000i128))) { panic!("{}", "pruv: requires violated: crate::evict_stop: ((-1000000000000000 <= limit) && (limit <= 1000000000000000))"); }
    evict_stop__pruv_body(bytes, free, limit)
}

}
#[allow(unused_imports)]
pub use self::pruv_impl::*;
