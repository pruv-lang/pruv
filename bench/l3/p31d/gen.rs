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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget { pub limit_ms: i64, pub active_ms: i64 }

#[allow(dead_code, non_camel_case_types, non_snake_case, private_interfaces)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock { pub since_ms: i64, pub armed: bool }

#[allow(dead_code, non_camel_case_types, non_snake_case, private_interfaces)]
#[derive(Clone, Debug)]
pub struct St { pub sessions: ::std::collections::HashMap<::std::string::String, Budget>, pub clocks: ::std::collections::HashMap<::std::string::String, Clock> }

#[allow(dead_code, non_camel_case_types, non_snake_case, private_interfaces)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effect { pub tag: i64, pub argument: i64 }

#[allow(dead_code, non_camel_case_types, non_snake_case, private_interfaces)]
#[derive(Clone, Debug)]
pub struct Reply { pub status: i64, pub effects: ::std::vec::Vec<Effect> }


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

#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct pruv_contract__Budget { limit_ms: i128, active_ms: i128 }
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables)]
#[inline(always)]
fn pruv_to_c__Budget(v: Budget) -> pruv_contract__Budget { pruv_contract__Budget { limit_ms: (v.limit_ms as i128), active_ms: (v.active_ms as i128) } }
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct pruv_contract__Clock { since_ms: i128, armed: bool }
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables)]
#[inline(always)]
fn pruv_to_c__Clock(v: Clock) -> pruv_contract__Clock { pruv_contract__Clock { since_ms: (v.since_ms as i128), armed: v.armed } }
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct pruv_contract__Effect { tag: i128, argument: i128 }
#[allow(dead_code, non_camel_case_types, non_snake_case, unused_variables)]
#[inline(always)]
fn pruv_to_c__Effect(v: Effect) -> pruv_contract__Effect { pruv_contract__Effect { tag: (v.tag as i128), argument: (v.argument as i128) } }

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
fn pruv_g6_vec_new__n10_1_6_Effect_0() -> ::std::vec::Vec<Effect> {
    let pruv_r: ::std::vec::Vec<Effect> = ::std::vec::Vec::<Effect>::new();
    pruv_r
}

#[allow(unused_mut, unused_variables, unused_parens, non_snake_case, non_camel_case_types, dead_code, unreachable_code, unreachable_patterns, unconditional_panic, arithmetic_overflow, unused_assignments, unused_comparisons)]
#[inline(always)]
fn pruv_g6_vec_push__n10_1_6_Effect_0(mut v: ::std::vec::Vec<Effect>, x: Effect) -> ::std::vec::Vec<Effect> {
    if !((v.len() as u64) < (1u64 << 40)) { pruv_g6_req("vec_push", "len(v) < 2^40"); }
    v.push(x);
    v
}

#[allow(unused_mut, unused_variables, unused_parens, non_snake_case, non_camel_case_types, dead_code, unreachable_code, unreachable_patterns, unconditional_panic, arithmetic_overflow, unused_assignments, unused_comparisons)]
#[inline(always)]
fn refresh__pruv_body(st: &mut St, sid: &str, bid: &str, now: i64) -> i64 {
    let mut res: i64 = 1i64;
    match st.sessions.get_mut(sid) {
        Some(budget) => {
            match st.clocks.get_mut(bid) {
                Some(clock) => {
                    if (((((0i64 <= (*clock).since_ms) && ((*clock).since_ms <= now)) && (now <= 1000000i64)) && (0i64 <= (*budget).active_ms)) && ((*budget).active_ms <= 1000000000000i64)) {
                        budget.active_ms = ((*budget).active_ms + (now - (*clock).since_ms));
                    }
                    clock.since_ms = now;
                    clock.armed = ((*budget).active_ms < (*budget).limit_ms);
                    res = 0i64;
                }
                None => {
                }
            }
        }
        None => {
        }
    }
    return res;
}

#[allow(unused_mut, unused_variables, unused_parens, non_snake_case, non_camel_case_types, dead_code, unreachable_code, unreachable_patterns, unconditional_panic, arithmetic_overflow, unused_assignments, unused_comparisons)]
#[inline(always)]
pub fn refresh(st: &mut St, sid: &str, bid: &str, now: i64) -> i64 {
    #[cold] #[inline(never)] fn pruv_len_slow(sid: usize, bid: usize) -> ! { if (sid as u64) > (1u64 << 40) { panic!("pruv: invalid representation: {} (str length)", sid) } if (bid as u64) > (1u64 << 40) { panic!("pruv: invalid representation: {} (str length)", bid) } unreachable!() }
    if ((sid.len() as u64) > (1u64 << 40)) | ((bid.len() as u64) > (1u64 << 40)) { pruv_len_slow(sid.len(), bid.len()) }
    refresh__pruv_body(st, sid, bid, now)
}

#[allow(unused_mut, unused_variables, unused_parens, non_snake_case, non_camel_case_types, dead_code, unreachable_code, unreachable_patterns, unconditional_panic, arithmetic_overflow, unused_assignments, unused_comparisons)]
#[inline(always)]
fn poll__pruv_body(st: &mut St, sid: &str) -> Reply {
    let mut status: i64 = 1i64;
    let mut effects: ::std::vec::Vec<Effect> = pruv_g6_vec_new__n10_1_6_Effect_0();
    match st.sessions.get_mut(sid) {
        Some(budget) => {
            status = 0i64;
            if ((*budget).active_ms >= (*budget).limit_ms) {
                effects = pruv_g6_vec_push__n10_1_6_Effect_0(effects, (Effect { tag: 1i64, argument: (*budget).active_ms }));
            }
        }
        None => {
        }
    }
    return (Reply { status: status, effects: effects });
}

#[allow(unused_mut, unused_variables, unused_parens, non_snake_case, non_camel_case_types, dead_code, unreachable_code, unreachable_patterns, unconditional_panic, arithmetic_overflow, unused_assignments, unused_comparisons)]
#[inline(always)]
pub fn poll(st: &mut St, sid: &str) -> Reply {
    #[cold] #[inline(never)] fn pruv_len_slow(sid: usize) -> ! { if (sid as u64) > (1u64 << 40) { panic!("pruv: invalid representation: {} (str length)", sid) } unreachable!() }
    if ((sid.len() as u64) > (1u64 << 40)) { pruv_len_slow(sid.len()) }
    poll__pruv_body(st, sid)
}

}
#[allow(unused_imports)]
pub use self::pruv_impl::*;
