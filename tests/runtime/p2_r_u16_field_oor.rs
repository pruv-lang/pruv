fn main() {
    println!("{}", sol::store(std::hint::black_box(65535)));
    println!("{}", sol::store(std::hint::black_box(65536)));
}
