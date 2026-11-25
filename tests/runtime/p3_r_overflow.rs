fn main() {
    println!("{}", sol::inc(std::hint::black_box(i64::MAX - 1)));
    println!("{}", sol::inc(std::hint::black_box(i64::MAX)));
}
