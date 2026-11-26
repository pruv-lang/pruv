fn main() {
    for k in [0i64, 3, 10] {
        println!("{}", sol::run(std::hint::black_box(k)));
    }
}
