fn main() {
    for c in [true, false] { println!("{}", sol::run(std::hint::black_box(c))); }
}
