fn main() {
    let b = std::hint::black_box;
    println!("{}", sol::f(b(7), b(2)));
    println!("{}", sol::f(b(-7), b(2)));
}
