fn main() {
    println!("{}", sol::take_ret("hello"));
    println!("{}", sol::take_ret("héllo wörld"));
    println!("{}", sol::take_ret(""));
    println!("{}", sol::take_ret("日本"));
    println!("{}", sol::copy_ret("hello"));
    println!("{}", sol::copy_ret(""));
    println!("{}", sol::take_let("héllo wörld"));
    println!("{}", sol::take_let(""));
    println!("{}", sol::take_let("日本"));
    println!("{}", sol::copy_let("日本"));
    println!("{}", sol::copy_let(""));
    println!("{}", sol::copy_then_move("abc"));
    println!("{}", sol::copy_then_move(""));
}
