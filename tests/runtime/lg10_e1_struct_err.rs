fn main() {
    println!("{}", sol::parse_or_code(55));
    println!("{}", sol::parse_or_code(63));
    println!("{}", sol::or_false(Ok(true)));
    println!("{}", sol::or_false(Err(sol::ParseErr { pos: 0, code: 1 })));
    println!("{}", sol::code(sol::parse_digit(63)));
}
