fn main() {
    println!("{}", sol::push(std::hint::black_box(65535)));
    println!("{}", sol::push(std::hint::black_box(65536)));
}
