fn main() {
    let b = std::hint::black_box;
    println!("{}", sol::f(b(46340), b(46340)));
    println!("{}", sol::f(b(1), b(2147483647)));
    println!("{}", sol::f(b(65536), b(32768)));
}
