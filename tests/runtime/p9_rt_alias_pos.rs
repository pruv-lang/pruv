fn main() {
    for i in [0, 5, 9] { println!("{}", sol::run(std::hint::black_box(i))); }
}
