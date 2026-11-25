fn main() {
    let b = std::hint::black_box;
    println!("{}", sol::f(b(3), b(0)));
    println!("{}", sol::f(b(4), b(0)));
}
