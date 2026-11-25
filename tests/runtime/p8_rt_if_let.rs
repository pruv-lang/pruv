fn main() {
    for k in [0, 1, 9] { println!("{}", sol::run(std::hint::black_box(k))); }
}
