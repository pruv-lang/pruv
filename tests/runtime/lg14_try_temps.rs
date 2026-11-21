fn main() {
    for x in [8, 6, 5, 0] {
        for k in 0..2 {
            println!("{}", sol::run(k, x));
        }
        println!("{}", sol::run_index(&[x, 7]));
    }
    println!("{}", sol::run_move(4));
    println!("{}", sol::run_move(0));
}
