fn main() {
    println!("{}", sol::inc(std::hint::black_box(65534i64)));
    println!("{}", sol::inc(std::hint::black_box(65535i64)));
}
