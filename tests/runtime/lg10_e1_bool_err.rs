fn main() {
    println!("{}", sol::status(sol::choose(true)));
    println!("{}", sol::status(sol::choose(false)));
    println!("{}", sol::status(Err(false)));
}
