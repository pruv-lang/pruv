fn main() {
    for i in [0, 2, 3, 9] { println!("{}", sol::run(std::hint::black_box(i))); }
}
