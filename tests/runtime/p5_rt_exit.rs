fn main() {
    for k in [0, 1, 3, 5, 7, 10] { println!("{}", sol::run(std::hint::black_box(k))); }
}
