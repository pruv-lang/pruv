fn main() {
    let lo = std::hint::black_box(0i64);
    let hi = std::hint::black_box(255i64);
    println!("{}", sol::edge(lo));
    println!("{}", sol::edge(hi));
}
