fn main() {
    for k in [0, 7, 1000] { println!("{}", sol::run(std::hint::black_box(k))); }
}
