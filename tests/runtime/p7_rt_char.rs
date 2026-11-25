fn main() {
    for k in [0, 3, 9] { println!("{}", sol::run(std::hint::black_box(k))); }
}
