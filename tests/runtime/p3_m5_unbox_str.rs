fn main() {
    println!("{}", sol::ret_word());
    println!("{}", sol::ret_empty());
    println!("{}", sol::ret_pair());
    println!("{}", sol::take_let());
    println!("{}", sol::empty_let());
    println!("{}", sol::early(1));
    println!("{}", sol::early(0));
    println!("{}", sol::copy_then_len());
    println!("{}", sol::empty_copy());
    println!("{}", sol::neg_byte());
}
