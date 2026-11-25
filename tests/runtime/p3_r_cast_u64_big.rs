fn main() {
    let b = std::hint::black_box;
    println!("{}", sol::f(b(1), b(1)));
    println!("{}", sol::f(b(4611686018427387903), b(1)));
    println!("{}", sol::f(b(4611686018427387904), b(0)));
}
