fn main() {
    println!("{}", sol::code(5));
    println!("{}", sol::code(-3));
    println!("{}", sol::code(500));
    println!("{}", sol::ignore_err(-1));
    println!("{}", sol::ignore_err(9));
    println!("{}", sol::unused(4));
    println!("{}", sol::unused(-4));
    println!("{}", sol::early(-2, true));
    println!("{}", sol::early(-2, false));
    println!("{}", sol::early(3, true));
    println!("{}", sol::parity(4));
    println!("{}", sol::parity(3));
    println!("{}", sol::parity(-3));
}
