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
#[allow(dead_code, non_camel_case_types, non_snake_case, private_interfaces)]
#[derive(Clone, Debug)]
pub struct Session { pub used: i64, pub stopped: ::std::vec::Vec<::std::string::String>, pub runs: ::std::collections::HashMap<::std::string::String, bool> }

#[allow(dead_code, non_camel_case_types, non_snake_case, private_interfaces)]
#[derive(Clone, Debug)]
pub struct St { pub sessions: ::std::collections::HashMap<::std::string::String, Session> }


#[allow(dead_code)]
#[inline(always)]
fn pruv_str_boundary(a: &str, i: i64) -> bool { i >= 0 && (i as u64) <= (a.len() as u64) && a.is_char_boundary(i as usize) }

::std::thread_local! { static PRUV_CTX: ::std::cell::Cell<&'static str> = ::std::cell::Cell::new(""); }
#[allow(dead_code)]
struct PruvCtx;
impl ::std::ops::Drop for PruvCtx { fn drop(&mut self) { PRUV_CTX.with(|c| c.set("")); } }
#[allow(dead_code)]
fn pruv_ctx(name: &'static str) -> PruvCtx { PRUV_CTX.with(|c| c.set(name)); PruvCtx }
#[allow(dead_code)]
#[cold]
#[inline(never)]
fn pruv_ud(what: &str) -> ! {
    let c: &'static str = PRUV_CTX.with(|c| c.get());
    if c.is_empty() { panic!("pruv: requires undefined: {}", what) } else { panic!("pruv: foreign contract undefined: {}: {}", c, what) }
}
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

#[allow(dead_code)]
#[inline(always)]
fn pruv_cidx_str(a: &str, i: i128) -> i128 { if i >= 0 && i < (a.len() as i128) { a.as_bytes()[i as usize] as i128 } else { pruv_ud("array index out of range") } }
#[allow(dead_code)]
#[inline(always)]
fn pruv_cboundary(a: &str, i: i128) -> bool { i >= 0 && i <= (a.len() as i128) && a.is_char_boundary(i as usize) }

#[allow(dead_code)]
#[cold]
#[inline(never)]
fn pruv_g6_req(name: &str, what: &str) -> ! { panic!("pruv: foreign requires violated: {}: {}", name, what) }
#[allow(dead_code)]
#[cold]
#[inline(never)]
fn pruv_g6_ens(name: &str, what: &str) -> ! { panic!("pruv: foreign ensures violated: {}: {}", name, what) }
#[allow(dead_code)]
#[cold]
#[inline(never)]
fn pruv_g6_repr(what: &str) -> ! { panic!("pruv: invalid representation: {}", what) }
#[allow(dead_code)]
#[inline(always)]
fn pruv_g6_charge(budget: &mut i64, n: usize) { let n: i64 = if n > (i64::MAX as usize) { i64::MAX } else { n as i64 }; *budget = budget.saturating_sub(n); if *budget < 0 { pruv_ud("budget") } }

#[allow(unused_mut, unused_variables, unused_parens, non_snake_case, non_camel_case_types, dead_code, unreachable_code, unreachable_patterns, unconditional_panic, arithmetic_overflow, unused_assignments, unused_comparisons)]
#[inline(always)]
fn pruv_g6_string_new() -> ::std::string::String {
    let pruv_r: ::std::string::String = ::std::string::String::new();
    pruv_r
}

#[allow(unused_mut, unused_variables, unused_parens, non_snake_case, non_camel_case_types, dead_code, unreachable_code, unreachable_patterns, unconditional_panic, arithmetic_overflow, unused_assignments, unused_comparisons)]
#[inline(always)]
fn pruv_g6_string_push_str(mut s: ::std::string::String, text: &str) -> ::std::string::String {
    if !((s.len() as u64) + (text.len() as u64) <= (1u64 << 40)) { pruv_g6_req("string_push_str", "len(s) + len(text) <= 2^40"); }
    s.push_str(text);
    s
}

#[allow(unused_mut, unused_variables, unused_parens, non_snake_case, non_camel_case_types, dead_code, unreachable_code, unreachable_patterns, unconditional_panic, arithmetic_overflow, unused_assignments, unused_comparisons)]
#[inline(always)]
fn charge__pruv_body(st: &mut St, sid: &str, x: i64) -> i64 {
    let mut res: i64 = 1i64;
    match st.sessions.get_mut(sid) {
        Some(r) => {
            if (r.used <= 1000000000000i64) {
                r.used = (r.used + x);
                res = 0i64;
            } else {
                res = 2i64;
            }
        }
        None => {
        }
    }
    return res;
}

#[allow(unused_mut, unused_variables, unused_parens, non_snake_case, non_camel_case_types, dead_code, unreachable_code, unreachable_patterns, unconditional_panic, arithmetic_overflow, unused_assignments, unused_comparisons)]
#[inline(always)]
pub fn charge(st: &mut St, sid: &str, x: i64) -> i64 {
    #[cold] #[inline(never)] fn pruv_len_slow(sid: usize) -> ! { if (sid as u64) > (1u64 << 40) { panic!("pruv: invalid representation: {} (str length)", sid) } unreachable!() }
    if ((sid.len() as u64) > (1u64 << 40)) { pruv_len_slow(sid.len()) }
    let mut pruv_budget: i64 = 1_073_741_824;
    if !((((x as i128) >= 0i128) && ((x as i128) <= 1000000i128))) { panic!("{}", "pruv: requires violated: crate::charge: ((x >= 0) && (x <= 1000000))"); }
    charge__pruv_body(st, sid, x)
}

#[allow(unused_mut, unused_variables, unused_parens, non_snake_case, non_camel_case_types, dead_code, unreachable_code, unreachable_patterns, unconditional_panic, arithmetic_overflow, unused_assignments, unused_comparisons)]
#[inline(always)]
fn halt__pruv_body(st: &mut St, sid: &str, name: &str) -> i64 {
    let mut res: i64 = 1i64;
    match st.sessions.get_mut(sid) {
        Some(r) => {
            if ((r.stopped.len() as i64) < 1000000i64) {
                let gone: bool = r.runs.remove(name).is_some();
                let mut owned: ::std::string::String = pruv_g6_string_new();
                owned = pruv_g6_string_push_str(owned, name);
                r.stopped.push(owned);
                if gone {
                    res = 0i64;
                } else {
                    res = 3i64;
                }
            } else {
                res = 2i64;
            }
        }
        None => {
        }
    }
    return res;
}

#[allow(unused_mut, unused_variables, unused_parens, non_snake_case, non_camel_case_types, dead_code, unreachable_code, unreachable_patterns, unconditional_panic, arithmetic_overflow, unused_assignments, unused_comparisons)]
#[inline(always)]
pub fn halt(st: &mut St, sid: &str, name: &str) -> i64 {
    #[cold] #[inline(never)] fn pruv_len_slow(sid: usize, name: usize) -> ! { if (sid as u64) > (1u64 << 40) { panic!("pruv: invalid representation: {} (str length)", sid) } if (name as u64) > (1u64 << 40) { panic!("pruv: invalid representation: {} (str length)", name) } unreachable!() }
    if ((sid.len() as u64) > (1u64 << 40)) | ((name.len() as u64) > (1u64 << 40)) { pruv_len_slow(sid.len(), name.len()) }
    halt__pruv_body(st, sid, name)
}

}
#[allow(unused_imports)]
pub use self::pruv_impl::*;
