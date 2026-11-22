fn main() {
    for x in [5, 0] {
        for k in 0..5 {
            println!("{}", sol::run(k, x));
        }
    }
}
