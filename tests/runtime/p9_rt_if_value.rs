fn main() {
    for i in [0, 1, 2, 5] { println!("{}", sol::run(std::hint::black_box(i))); }
}
