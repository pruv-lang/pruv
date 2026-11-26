fn main() {
    for (w, h) in [(6i64, 7i64), (0, 0), (1000, 1000)] {
        println!("{}", sol::area(std::hint::black_box(w), h));
    }
}
