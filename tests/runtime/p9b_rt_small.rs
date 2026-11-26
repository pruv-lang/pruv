fn main() {
    for k in [1i64, 5, 8] {
        println!("{}", sol::run(std::hint::black_box(k)));
    }
}
