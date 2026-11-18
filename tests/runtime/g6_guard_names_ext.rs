// P19c regression fixture (rlib g6_guard_names): parameter names that look like the old guard type name.
#![allow(non_snake_case)]

pub fn guard_add(PruvForeignGuard: i64, y: i64) -> i64 {
    PruvForeignGuard.wrapping_add(y)
}

pub fn guard_boom(PruvForeignGuard: i64) -> i64 {
    if PruvForeignGuard >= 0 {
        panic!("guard boom");
    }
    PruvForeignGuard
}
