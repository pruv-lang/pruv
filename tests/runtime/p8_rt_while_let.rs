fn main() {
    for k in [0, 1, 2, 3, 4, 5, 6] {
        let row: Vec<String> = (0..3).map(|p| sol::run(std::hint::black_box(k), p).to_string()).collect();
        println!("{}", row.join(" "));
    }
}
