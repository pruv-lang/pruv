fn main() {
    for k in [0i64, 7, 50] {
        println!("{}", sol::run(std::hint::black_box(k)));
    }
}
