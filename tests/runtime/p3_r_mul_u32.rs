fn main() {
    let b = std::hint::black_box;
    println!("{}", sol::f(b(65535), b(65537)));
    println!("{}", sol::f(b(65536), b(65536)));
}
