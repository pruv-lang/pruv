fn main() {
    println!("{}", sol::narrow(std::hint::black_box(-32768i64)));
    println!("{}", sol::narrow(std::hint::black_box(-32769i64)));
}
