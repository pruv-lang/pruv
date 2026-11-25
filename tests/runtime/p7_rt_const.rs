fn main() {
    for k in [0, 5] { println!("{}", sol::run(std::hint::black_box(k))); }
}
