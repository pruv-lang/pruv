fn main() {
    for k in [0, 3] { println!("{}", sol::run(std::hint::black_box(k))); }
}
