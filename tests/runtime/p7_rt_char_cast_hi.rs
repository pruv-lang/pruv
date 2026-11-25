fn main() {
    println!("{}", sol::conv(std::hint::black_box(0xE000i64)));
    println!("{}", sol::conv(std::hint::black_box(0x110000i64)));
}
