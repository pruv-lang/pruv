fn main() {
    let a = sol::take(Ok(()));
    let b = sol::take(Err(42));
    let c: Result<(), i64> = sol::gate(3);
    let d: Result<(), i64> = sol::gate(-9);
    let e = sol::only_ok(Ok(()), Err(4));
    println!("{}", sol::checksum());
    println!("{} {} {:?} {:?} {}", a, b, c, d, e);
}
