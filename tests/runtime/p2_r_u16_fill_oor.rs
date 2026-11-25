fn main() {
    println!("{}", sol::widen(std::hint::black_box(65535)));
    println!("{}", sol::widen(std::hint::black_box(-1)));
}
