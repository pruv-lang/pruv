fn main() {
    for n in [0i64, 3, 9] {
        println!("{}", sol::run(std::hint::black_box(n)));
    }
}
