fn main() {
    let b = std::hint::black_box;
    println!("{}", sol::f(b(3), b(1)));
    println!("{}", sol::f(b(0), b(1)));
}
