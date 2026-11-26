fn main() {
    for i in [4, -5, -1] { println!("{}", sol::run(std::hint::black_box(i))); }
}
