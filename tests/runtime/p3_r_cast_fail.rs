fn main() {
    println!("{}", sol::narrow(std::hint::black_box(4294967295i64)));
    println!("{}", sol::narrow(std::hint::black_box(4294967296i64)));
}
