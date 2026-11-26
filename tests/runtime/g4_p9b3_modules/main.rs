fn main() {
    for k in [0i64, 5, 1000] {
        println!("{}", sol::total(std::hint::black_box(k)));
    }
}
