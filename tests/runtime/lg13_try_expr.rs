fn main() {
    for x in [8, 6, 0, 200, 5, 20] {
        for k in 0..6 {
            println!("{}", sol::run(k, x));
        }
    }
    println!("{}", sol::run_pos(3, 4));
    println!("{}", sol::run_pos(3, 0));
    println!("{}", sol::run_pos(0, 4));
}
