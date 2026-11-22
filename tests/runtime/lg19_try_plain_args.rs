fn main() {
    for x in [5, 0] {
        for k in 0..3 {
            println!("{}", sol::run(k, x));
        }
    }
}
