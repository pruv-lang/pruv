fn main() {
    for i in [3, -2, 0] { println!("{}", sol::run(std::hint::black_box(i))); }
}
