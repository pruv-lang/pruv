fn main() {
    for i in [0, 1, 3] { println!("{}", sol::run(std::hint::black_box(i))); }
}
