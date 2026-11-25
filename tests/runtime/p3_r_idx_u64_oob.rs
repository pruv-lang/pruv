fn main() {
    let b = std::hint::black_box;
    println!("{}", sol::f(b(1), b(0)));
    println!("{}", sol::f(b(i64::MAX), b(0)));
}
