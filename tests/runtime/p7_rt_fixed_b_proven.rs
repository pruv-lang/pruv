fn main() {
    for i in [0, 1, 2] { println!("{}", sol::run(std::hint::black_box(i))); }
}
