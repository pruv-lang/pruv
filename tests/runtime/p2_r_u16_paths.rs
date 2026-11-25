fn main() {
    println!("{}", sol::run(std::hint::black_box(0)));
    println!("{}", sol::run(std::hint::black_box(65535)));
}
