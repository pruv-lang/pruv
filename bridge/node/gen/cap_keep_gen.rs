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
fn cap_keep__pruv_body(lens: &[i64], max: i64) -> i64 {
    let mut used: i64 = 0i64;
    let mut i: i64 = 0i64;
    while (i < (lens.len() as i64)) {
        used = (used + (unsafe { *lens.get_unchecked((i) as usize) }));
        if (used > max) {
            return i;
        }
        i = (i + 1i64);
    }
    return (lens.len() as i64);
}

#[allow(unused_mut, unused_variables, unused_parens, non_snake_case, non_camel_case_types, dead_code, unreachable_code, unreachable_patterns, unconditional_panic, arithmetic_overflow)]
#[inline(always)]
pub fn cap_keep(lens: &[i64], max: i64) -> i64 {
    #[cold] #[inline(never)] fn pruv_len_slow(lens: usize) -> ! { if (lens as u64) > (1u64 << 40) { panic!("pruv: invalid representation: {} (array length)", lens) } unreachable!() }
    if ((lens.len() as u64) > (1u64 << 40)) { pruv_len_slow(lens.len()) }
    let mut pruv_budget: i64 = 1_073_741_824;
    if !(((lens.len() as i128) <= 10000000i128)) { panic!("{}", "pruv: requires violated: crate::cap_keep: (len(lens) <= 10000000)"); }
    if !({ let pruv_lo1: i128 = 0i128; let pruv_hi1: i128 = (lens.len() as i128); let mut pruv_k1: i128 = pruv_lo1; let mut pruv_r1 = true; if !(pruv_lo1 >= pruv_hi1 || (pruv_lo1 >= -9223372036854775808i128 && pruv_hi1 <= 9223372036854775807i128 && pruv_hi1 - pruv_lo1 <= (pruv_budget as i128) && pruv_lo1 >= 0i128 && pruv_hi1 <= (lens.len() as i128) && { let mut pruv_a1 = true; let mut pruv_j1: i64 = pruv_lo1 as i64; let pruv_e1: i64 = pruv_hi1 as i64; while pruv_j1 < pruv_e1 { let k: i128 = pruv_j1 as i128; pruv_a1 &= ((0i128 <= ((unsafe { *lens.get_unchecked(pruv_j1 as usize) }) as i128)) & (((unsafe { *lens.get_unchecked(pruv_j1 as usize) }) as i128) <= 1000000000i128)); pruv_j1 += 1; } pruv_a1 })) { while pruv_k1 < pruv_hi1 { pruv_budget -= 1; if pruv_budget < 0 { pruv_ud("contract iteration budget exceeded"); } let k: i128 = pruv_k1; if !(((0i128 <= pruv_cidx(lens, k)) && (pruv_cidx(lens, k) <= 1000000000i128))) { pruv_r1 = false; break; } pruv_k1 += 1; } } else if pruv_hi1 > pruv_lo1 { let pruv_b = &mut pruv_budget; *pruv_b -= (pruv_hi1 - pruv_lo1) as i64; } pruv_r1 }) { panic!("{}", "pruv: requires violated: crate::cap_keep: forall k in 0..len(lens): ((0 <= lens[k]) && (lens[k] <= 1000000000))"); }
    if !((((-1000000000000000i128) <= (max as i128)) && ((max as i128) <= 1000000000000000i128))) { panic!("{}", "pruv: requires violated: crate::cap_keep: ((-1000000000000000 <= max) && (max <= 1000000000000000))"); }
    cap_keep__pruv_body(lens, max)
}

}
#[allow(unused_imports)]
pub use self::pruv_impl::*;
