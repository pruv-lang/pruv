fn main() {
    println!("{}", sol::conv(std::hint::black_box(0x10FFFFi64)));
    println!("{}", sol::conv(std::hint::black_box(0xD7FFi64)));
    println!("{}", sol::conv(std::hint::black_box(0xD800i64)));
}
