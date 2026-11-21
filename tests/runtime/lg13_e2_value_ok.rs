fn main() {
    for x in [-2, 0, 7] {
        for k in 0..3 {
            println!("{}", sol::run(k, x));
        }
    }
}
